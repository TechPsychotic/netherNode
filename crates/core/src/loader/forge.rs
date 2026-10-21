//! Forge and NeoForge. Their installers carry a version JSON plus "processors" (small Java
//! tools that deobfuscate and patch the game jar). We run those processors ourselves instead
//! of the installer's own code, so it works with any Java and on every distro.
//!
//! In particular, Forge checks processor outputs byte-for-byte. Java builds that use the
//! system zlib on distros with zlib-ng (Fedora 40+, among others) produce identical content
//! with different compressed bytes, which makes the official installer abort. We only warn.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::{LoaderKind, LoaderVersion, maven_versions};
use crate::download::{self, Download, Progress, ProgressFn};
use crate::error::{Result, other};
use crate::java;
use crate::paths::{Paths, ensure_parent};
use crate::rules::Environment;
use crate::version::{self, JavaVersion, Library, maven_path};

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged";

/// Numeric parts of a version string, for sorting ("47.4.10" sorts after "47.4.9").
fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// NeoForge versions encode the Minecraft version: 1.21.1 -> "21.1.x", 26.3 -> "26.3.0.x".
fn neoforge_prefix(mc: &str) -> Option<String> {
    let parts: Vec<&str> = mc.split('.').collect();
    match parts.as_slice() {
        ["1", minor] => Some(format!("{minor}.0.")),
        ["1", minor, patch] => Some(format!("{minor}.{patch}.")),
        [year, drop] => Some(format!("{year}.{drop}.0.")),
        [year, drop, hotfix] => Some(format!("{year}.{drop}.{hotfix}.")),
        _ => None,
    }
}

pub async fn versions(
    http: &reqwest::Client,
    kind: LoaderKind,
    mc: &str,
) -> Result<Vec<LoaderVersion>> {
    let mut out: Vec<LoaderVersion> = if kind == LoaderKind::Forge {
        let xml = http
            .get(format!("{FORGE_MAVEN}/maven-metadata.xml"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let promos: Value = download::get_json(
            http,
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
        )
        .await
        .unwrap_or(Value::Null);
        let recommended = promos["promos"][format!("{mc}-recommended")]
            .as_str()
            .map(String::from);
        maven_versions(&xml)
            .into_iter()
            .filter_map(|v| v.strip_prefix(&format!("{mc}-")).map(String::from))
            .map(|v| LoaderVersion {
                stable: recommended.as_deref() == Some(v.as_str()),
                version: v,
            })
            .collect()
    } else if mc == "1.20.1" {
        // NeoForge's first release kept Forge's coordinates and numbering.
        let xml = http
            .get(format!("{NEOFORGE_MAVEN}/forge/maven-metadata.xml"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        maven_versions(&xml)
            .into_iter()
            .filter_map(|v| v.strip_prefix("1.20.1-").map(String::from))
            .map(|v| LoaderVersion {
                stable: !v.contains('-'),
                version: v,
            })
            .collect()
    } else {
        let Some(prefix) = neoforge_prefix(mc) else {
            return Ok(Vec::new());
        };
        let xml = http
            .get(format!("{NEOFORGE_MAVEN}/neoforge/maven-metadata.xml"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        maven_versions(&xml)
            .into_iter()
            .filter(|v| v.starts_with(&prefix))
            .map(|v| LoaderVersion {
                stable: !v.contains('-'),
                version: v,
            })
            .collect()
    };
    out.sort_by_key(|v| std::cmp::Reverse(version_key(&v.version)));
    Ok(out)
}

fn installer_url(kind: LoaderKind, mc: &str, version: &str) -> (String, String) {
    match kind {
        LoaderKind::Forge => {
            let full = format!("{mc}-{version}");
            let name = format!("forge-{full}-installer.jar");
            (format!("{FORGE_MAVEN}/{full}/{name}"), name)
        }
        _ if mc == "1.20.1" => {
            let full = format!("1.20.1-{version}");
            let name = format!("forge-{full}-installer.jar");
            (
                format!("{NEOFORGE_MAVEN}/forge/{full}/{name}"),
                format!("neo{name}"),
            )
        }
        _ => {
            let name = format!("neoforge-{version}-installer.jar");
            (format!("{NEOFORGE_MAVEN}/neoforge/{version}/{name}"), name)
        }
    }
}

type Zip = zip::ZipArchive<std::fs::File>;

fn read_entry(zip: &mut Zip, name: &str) -> Result<Vec<u8>> {
    let mut entry = zip.by_name(name.trim_start_matches('/'))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn extract_entry(zip: &mut Zip, name: &str, dest: &Path) -> Result<()> {
    let bytes = read_entry(zip, name)?;
    ensure_parent(dest)?;
    std::fs::write(dest, bytes)?;
    Ok(())
}

fn artifact_path(paths: &Paths, coordinate: &str) -> Result<PathBuf> {
    let rel = maven_path(coordinate, None)
        .ok_or_else(|| other(format!("bad Maven coordinate '{coordinate}'")))?;
    Ok(paths.libraries().join(rel))
}

pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    kind: LoaderKind,
    mc: &str,
    version: &str,
    progress: &ProgressFn,
) -> Result<String> {
    let (url, name) = installer_url(kind, mc, version);
    let installer = paths.root.join("cache").join("installers").join(name);
    if !installer.exists() {
        progress(Progress::Stage {
            name: format!("{kind} installer"),
            total: 1,
        });
        download::fetch(
            http,
            &Download {
                url,
                dest: installer.clone(),
                sha1: None,
                size: None,
            },
        )
        .await?;
        progress(Progress::Advance);
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&installer)?)?;
    let profile: Value = serde_json::from_slice(&read_entry(&mut zip, "install_profile.json")?)?;

    if profile.get("versionInfo").is_some() {
        return install_legacy(paths, mc, &profile, &mut zip);
    }

    let json_name = profile["json"].as_str().unwrap_or("/version.json");
    let mut version_json: Value = serde_json::from_slice(&read_entry(&mut zip, json_name)?)?;
    let id = version_json["id"]
        .as_str()
        .ok_or_else(|| other("installer version.json has no id"))?
        .to_string();
    let done_marker = paths.version_json(&id);
    if done_marker.exists() {
        return Ok(id);
    }

    // Artifacts shipped inside the installer (e.g. Forge itself) live under maven/.
    let bundled: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| n.starts_with("maven/") && !n.ends_with('/'))
        .collect();
    for name in bundled {
        let dest = paths.libraries().join(name.trim_start_matches("maven/"));
        if !dest.exists() {
            extract_entry(&mut zip, &name, &dest)?;
        }
    }

    // Vanilla client jar: the processors read it.
    let vanilla = version::resolve(http, paths, mc).await?;
    if let Some(client) = vanilla.downloads.as_ref().and_then(|d| d.client.clone()) {
        let jar = Download {
            url: client.url,
            dest: paths.version_jar(mc),
            sha1: client.sha1,
            size: client.size,
        };
        download::fetch_all(http, "Minecraft", vec![jar], progress).await?;
    }

    // Libraries for both the processors and the game.
    let env = Environment::current();
    let mut libraries: Vec<Library> =
        serde_json::from_value(profile["libraries"].clone()).unwrap_or_default();
    libraries.extend(
        serde_json::from_value::<Vec<Library>>(version_json["libraries"].clone())
            .unwrap_or_default(),
    );
    let downloads: Vec<Download> = libraries
        .iter()
        .filter(|l| crate::rules::allowed(&l.rules, &env))
        .filter_map(|l| l.artifact())
        .filter(|a| !a.url.is_empty())
        .map(|a| Download {
            dest: paths
                .libraries()
                .join(a.path.as_deref().unwrap_or_default()),
            url: a.url,
            sha1: a.sha1,
            size: a.size,
        })
        .collect();
    download::fetch_all(http, &format!("{kind} libraries"), downloads, progress).await?;

    let required = vanilla.java_version.clone().unwrap_or(JavaVersion {
        component: Some("jre-legacy".into()),
        major_version: 8,
    });
    let java = java::resolve(http, paths, &env, &required, progress)
        .await?
        .java;
    let work = paths.root.join("cache").join("forge-work").join(&id);
    run_processors(
        paths, mc, &installer, &mut zip, &profile, &java, &work, progress,
    )
    .await?;
    let _ = std::fs::remove_dir_all(&work);

    // Mark installed last, so an interrupted install is retried.
    if version_json.get("inheritsFrom").is_none() {
        version_json["inheritsFrom"] = Value::String(mc.into());
    }
    ensure_parent(&done_marker)?;
    std::fs::write(&done_marker, serde_json::to_vec_pretty(&version_json)?)?;
    Ok(id)
}

/// Pre-1.12.2 installers: the whole version JSON is in `versionInfo`, and the universal
/// jar sits at the installer's root.
fn install_legacy(paths: &Paths, mc: &str, profile: &Value, zip: &mut Zip) -> Result<String> {
    let mut info = profile["versionInfo"].clone();
    let id = info["id"]
        .as_str()
        .ok_or_else(|| other("installer versionInfo has no id"))?
        .to_string();
    if let (Some(path), Some(file)) = (
        profile["install"]["path"].as_str(),
        profile["install"]["filePath"].as_str(),
    ) {
        let dest = artifact_path(paths, path)?;
        if !dest.exists() {
            extract_entry(zip, file, &dest)?;
        }
    }
    // Old profiles repeat vanilla's libraries but don't say which jar to run.
    if info.get("inheritsFrom").is_none() {
        info["inheritsFrom"] = Value::String(mc.into());
    }
    let file = paths.version_json(&id);
    ensure_parent(&file)?;
    std::fs::write(&file, serde_json::to_vec_pretty(&info)?)?;
    Ok(id)
}

#[derive(Debug, Deserialize)]
struct Processor {
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
    sides: Option<Vec<String>>,
}

/// Expand `{KEY}` tokens and `'literal'` quotes the way Forge's installer does.
fn replace_tokens(s: &str, data: &HashMap<String, String>) -> Result<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            '{' => {
                let key: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let value = data
                    .get(&key)
                    .ok_or_else(|| other(format!("installer refers to unknown value {{{key}}}")))?;
                out.push_str(value);
            }
            '\'' => out.extend(chars.by_ref().take_while(|&c| c != '\'')),
            c => out.push(c),
        }
    }
    Ok(out)
}

/// `[group:artifact:version]` means "the path of that library"; anything else gets tokens expanded.
fn resolve_arg(paths: &Paths, arg: &str, data: &HashMap<String, String>) -> Result<String> {
    match arg.strip_prefix('[').and_then(|a| a.strip_suffix(']')) {
        Some(coordinate) => Ok(artifact_path(paths, coordinate)?
            .to_string_lossy()
            .into_owned()),
        None => replace_tokens(arg, data),
    }
}

fn main_class(jar: &Path) -> Result<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
    let manifest =
        String::from_utf8_lossy(&read_entry(&mut zip, "META-INF/MANIFEST.MF")?).into_owned();
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("Main-Class:"))
        .map(|m| m.trim().to_string())
        .ok_or_else(|| other(format!("{} has no Main-Class", jar.display())))
}

fn outputs_ok(outputs: &HashMap<PathBuf, String>) -> bool {
    !outputs.is_empty()
        && outputs.iter().all(|(file, sha1)| {
            download::sha1_file(file).is_ok_and(|h| h.eq_ignore_ascii_case(sha1))
        })
}

#[allow(clippy::too_many_arguments)]
async fn run_processors(
    paths: &Paths,
    mc: &str,
    installer: &Path,
    zip: &mut Zip,
    profile: &Value,
    java: &Path,
    work: &Path,
    progress: &ProgressFn,
) -> Result<()> {
    let processors: Vec<Processor> =
        serde_json::from_value(profile["processors"].clone()).unwrap_or_default();
    let processors: Vec<Processor> = processors
        .into_iter()
        .filter(|p| {
            p.sides
                .as_ref()
                .is_none_or(|s| s.iter().any(|s| s == "client"))
        })
        .collect();
    if processors.is_empty() {
        return Ok(());
    }

    let path_str = |p: PathBuf| p.to_string_lossy().into_owned();
    let mut data: HashMap<String, String> = HashMap::from([
        ("SIDE".into(), "client".into()),
        ("MINECRAFT_JAR".into(), path_str(paths.version_jar(mc))),
        ("MINECRAFT_VERSION".into(), mc.into()),
        ("ROOT".into(), path_str(paths.root.clone())),
        ("INSTALLER".into(), path_str(installer.to_path_buf())),
        ("LIBRARY_DIR".into(), path_str(paths.libraries())),
    ]);
    if let Some(entries) = profile["data"].as_object() {
        for (key, sides) in entries {
            let Some(value) = sides["client"].as_str() else {
                continue;
            };
            let resolved = if let Some(coordinate) =
                value.strip_prefix('[').and_then(|v| v.strip_suffix(']'))
            {
                path_str(artifact_path(paths, coordinate)?)
            } else if let Some(literal) =
                value.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))
            {
                literal.to_string()
            } else if value.starts_with('/') {
                // A file inside the installer (e.g. the binary patches): extract it.
                let dest = work.join(value.trim_start_matches('/'));
                extract_entry(zip, value, &dest)?;
                path_str(dest)
            } else {
                value.to_string()
            };
            data.insert(key.clone(), resolved);
        }
    }

    let separator = if cfg!(windows) { ";" } else { ":" };
    progress(Progress::Stage {
        name: "Patching Minecraft".into(),
        total: processors.len() as u64,
    });
    for processor in processors {
        let outputs: HashMap<PathBuf, String> = processor
            .outputs
            .iter()
            .map(|(k, v)| {
                Ok((
                    PathBuf::from(resolve_arg(paths, k, &data)?),
                    resolve_arg(paths, v, &data)?,
                ))
            })
            .collect::<Result<_>>()?;
        if outputs_ok(&outputs) {
            progress(Progress::Advance);
            continue;
        }

        let jar = artifact_path(paths, &processor.jar)?;
        let mut classpath = vec![path_str(jar.clone())];
        for entry in &processor.classpath {
            classpath.push(path_str(artifact_path(paths, entry)?));
        }
        let args: Vec<String> = processor
            .args
            .iter()
            .map(|a| resolve_arg(paths, a, &data))
            .collect::<Result<_>>()?;

        let mut cmd = tokio::process::Command::new(java);
        cmd.arg("-cp")
            .arg(classpath.join(separator))
            .arg(main_class(&jar)?)
            .args(&args)
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let out = cmd.output().await?;
        if !out.status.success() {
            let log = String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = log
                .lines()
                .rev()
                .take(15)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            return Err(other(format!(
                "{} failed:\n{}",
                processor.jar,
                tail.join("\n")
            )));
        }
        for (file, sha1) in &outputs {
            match download::sha1_file(file) {
                Err(_) => {
                    return Err(other(format!(
                        "{} did not produce {}",
                        processor.jar,
                        file.display()
                    )));
                }
                // Same content, different compression (see the module docs). Safe to use.
                Ok(actual) if !actual.eq_ignore_ascii_case(sha1) => {
                    eprintln!(
                        "note: {} differs from Forge's checksum (expected with zlib-ng); continuing",
                        file.display()
                    )
                }
                Ok(_) => {}
            }
        }
        progress(Progress::Advance);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_prefixes() {
        assert_eq!(neoforge_prefix("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neoforge_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix("26.3").as_deref(), Some("26.3.0."));
        assert_eq!(neoforge_prefix("26.1.2").as_deref(), Some("26.1.2."));
    }

    #[test]
    fn tokens_and_quotes() {
        let data = HashMap::from([("SIDE".to_string(), "client".to_string())]);
        assert_eq!(
            replace_tokens("--side {SIDE}", &data).unwrap(),
            "--side client"
        );
        assert_eq!(replace_tokens("'{SIDE}'", &data).unwrap(), "{SIDE}");
        assert_eq!(replace_tokens(r"a\{b", &data).unwrap(), "a{b");
        assert!(replace_tokens("{MISSING}", &data).is_err());
    }

    #[test]
    fn version_sorting() {
        let mut v = ["47.4.9", "47.4.10", "47.3.0"];
        v.sort_by_key(|s| std::cmp::Reverse(version_key(s)));
        assert_eq!(v, ["47.4.10", "47.4.9", "47.3.0"]);
    }
}
