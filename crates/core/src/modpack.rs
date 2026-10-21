//! Modrinth modpacks (`.mrpack`): import from a file, or install straight from Modrinth.
//!
//! An `.mrpack` is a zip holding `modrinth.index.json` (files to download, with hashes, plus
//! the Minecraft and loader versions) and `overrides/` + `client-overrides/` folders that are
//! copied into the game directory. Following the format's rules, downloads are only accepted
//! from Modrinth/GitHub/GitLab hosts and every path must stay inside the instance.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{self, Download, Progress, ProgressFn};
use crate::error::{Result, other};
use crate::instance::Instance;
use crate::loader::{self, LoaderKind};
use crate::paths::{Paths, ensure_parent};

const MODRINTH: &str = "https://api.modrinth.com/v2";
const ALLOWED_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];

/// Which modpack an instance came from (for display, and future updates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModpackInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    format_version: u32,
    game: String,
    version_id: String,
    name: String,
    #[serde(default)]
    files: Vec<PackFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    path: String,
    hashes: HashMap<String, String>,
    env: Option<HashMap<String, String>>,
    downloads: Vec<String>,
    file_size: Option<u64>,
}

/// A relative path that stays inside the instance: no `..`, root or drive prefix.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    let ok = !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)));
    ok.then(|| p.to_path_buf())
}

fn allowed_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', ':']).next().unwrap_or_default();
    ALLOWED_HOSTS.contains(&host)
}

/// Loader named in the pack's dependencies, if any.
fn pack_loader(deps: &HashMap<String, String>) -> Option<(LoaderKind, String)> {
    [
        ("fabric-loader", LoaderKind::Fabric),
        ("quilt-loader", LoaderKind::Quilt),
        ("forge", LoaderKind::Forge),
        ("neoforge", LoaderKind::NeoForge),
    ]
    .into_iter()
    .find_map(|(key, kind)| deps.get(key).map(|v| (kind, v.clone())))
}

/// A free instance name based on `base`: "Pack", then "Pack (2)", "Pack (3)"...
fn free_name(paths: &Paths, base: &str) -> String {
    // Keep only characters that are valid in folder names on every OS.
    let clean: String = base
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let clean = clean
        .trim()
        .trim_end_matches('.')
        .chars()
        .take(48)
        .collect::<String>();
    let clean = if clean.is_empty() {
        "Modpack".to_string()
    } else {
        clean
    };
    (1..)
        .map(|n| {
            if n == 1 {
                clean.clone()
            } else {
                format!("{clean} ({n})")
            }
        })
        .find(|name| !paths.instances().join(name).exists())
        .expect("some name is free")
}

/// Import an `.mrpack` file as a new instance (named after the pack unless `name` is given).
pub async fn import(
    http: &reqwest::Client,
    paths: &Paths,
    mrpack: &Path,
    name: Option<&str>,
    project_id: Option<String>,
    progress: &ProgressFn,
) -> Result<Instance> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(mrpack)?)?;
    let index: Index = {
        let mut entry = zip
            .by_name("modrinth.index.json")
            .map_err(|_| other("not a Modrinth modpack (modrinth.index.json is missing)"))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)?
    };
    if index.game != "minecraft" || index.format_version != 1 {
        return Err(other(format!(
            "unsupported modpack (game {}, format {})",
            index.game, index.format_version
        )));
    }
    let mc = index
        .dependencies
        .get("minecraft")
        .ok_or_else(|| other("the modpack doesn't say which Minecraft version it needs"))?
        .clone();

    let instance_name = match name {
        Some(n) => n.to_string(),
        None => free_name(paths, &index.name),
    };
    let mut instance = Instance::create(paths, &instance_name, &mc, false)?;
    // Hidden until complete. If we fail, clean up now; if the process dies, the next
    // start's Instance::clean_interrupted does.
    let guard = instance.begin_install(paths)?;
    match fill(
        http,
        paths,
        &mut instance,
        &mut zip,
        &index,
        &mc,
        project_id,
        progress,
    )
    .await
    {
        Ok(()) => {
            guard.done()?;
            Ok(instance)
        }
        Err(e) => {
            drop(guard);
            let _ = std::fs::remove_dir_all(instance.dir(paths));
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn fill(
    http: &reqwest::Client,
    paths: &Paths,
    instance: &mut Instance,
    zip: &mut zip::ZipArchive<std::fs::File>,
    index: &Index,
    mc: &str,
    project_id: Option<String>,
    progress: &ProgressFn,
) -> Result<()> {
    if let Some((kind, version)) = pack_loader(&index.dependencies) {
        loader::apply(http, paths, instance, kind, Some(&version), progress).await?;
    }

    let game_dir = instance.game_dir(paths);
    let mut downloads = Vec::new();
    for file in &index.files {
        if file
            .env
            .as_ref()
            .and_then(|e| e.get("client"))
            .is_some_and(|c| c == "unsupported")
        {
            continue;
        }
        let rel = safe_relative(&file.path)
            .ok_or_else(|| other(format!("modpack file has an unsafe path: {}", file.path)))?;
        let url = file
            .downloads
            .iter()
            .find(|u| allowed_url(u))
            .ok_or_else(|| other(format!("{} has no download from a trusted host", file.path)))?;
        downloads.push(Download {
            url: url.clone(),
            dest: game_dir.join(rel),
            sha1: file.hashes.get("sha1").cloned(),
            size: file.file_size,
        });
    }
    download::fetch_all(http, "Modpack files", downloads, progress).await?;

    // overrides/ first, then client-overrides/ on top.
    for prefix in ["overrides/", "client-overrides/"] {
        let names: Vec<String> = zip
            .file_names()
            .filter_map(|n| n.ok().map(|n| n.into_owned()))
            .filter(|n| n.starts_with(prefix) && !n.ends_with('/'))
            .collect();
        if !names.is_empty() {
            progress(Progress::Stage {
                name: "Pack settings".into(),
                total: names.len() as u64,
            });
        }
        for name in names {
            let Some(rel) = safe_relative(&name[prefix.len()..]) else {
                return Err(other(format!(
                    "modpack override has an unsafe path: {name}"
                )));
            };
            let dest = game_dir.join(rel);
            ensure_parent(&dest)?;
            let mut entry = zip.by_name(&name)?;
            std::io::copy(&mut entry, &mut std::fs::File::create(&dest)?)?;
            progress(Progress::Advance);
        }
    }

    instance.modpack = Some(ModpackInfo {
        name: index.name.clone(),
        version: index.version_id.clone(),
        project_id,
    });
    instance.minecraft.get_or_insert_with(|| mc.to_string());
    instance.save(paths)?;
    // Show real mod names in the Mods screen; not fatal if Modrinth can't tell.
    let _ = crate::mods::identify(http, paths, instance).await;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct PackVersion {
    version_number: String,
    #[serde(default)]
    version_type: String,
    files: Vec<PackVersionFile>,
}

#[derive(Debug, Deserialize)]
struct PackVersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
    hashes: HashMap<String, String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Project {
    id: String,
    title: String,
}

/// Search Modrinth modpacks.
pub async fn search(
    http: &reqwest::Client,
    query: &str,
    offset: u32,
) -> Result<crate::mods::SearchResults> {
    let index = if query.trim().is_empty() {
        "downloads"
    } else {
        "relevance"
    };
    let resp = http
        .get(format!("{MODRINTH}/search"))
        .query(&[
            ("query", query),
            ("facets", r#"[["project_type:modpack"]]"#),
            ("index", index),
            ("limit", "20"),
            ("offset", &offset.to_string()),
        ])
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

/// Download a Modrinth modpack (newest release, else newest) and import it.
pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    project: &str,
    progress: &ProgressFn,
) -> Result<Instance> {
    let info: Project = download::get_json(http, &format!("{MODRINTH}/project/{project}")).await?;
    let versions: Vec<PackVersion> =
        download::get_json(http, &format!("{MODRINTH}/project/{}/version", info.id)).await?;
    let pick = versions
        .iter()
        .position(|v| v.version_type == "release")
        .unwrap_or(0);
    let version = versions
        .get(pick)
        .ok_or_else(|| other(format!("{} has no versions to install", info.title)))?;
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or(version.files.first())
        .filter(|f| f.filename.ends_with(".mrpack"))
        .ok_or_else(|| {
            other(format!(
                "{} {} has no .mrpack file",
                info.title, version.version_number
            ))
        })?;

    let dest = paths
        .root
        .join("cache")
        .join("modpacks")
        .join(&file.filename);
    progress(Progress::Stage {
        name: format!("{} {}", info.title, version.version_number),
        total: 1,
    });
    download::fetch(
        http,
        &Download {
            url: file.url.clone(),
            dest: dest.clone(),
            sha1: file.hashes.get("sha1").cloned(),
            size: file.size,
        },
    )
    .await?;
    progress(Progress::Advance);
    let instance = import(http, paths, &dest, None, Some(info.id), progress).await;
    let _ = std::fs::remove_file(&dest);
    instance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_the_instance() {
        assert!(safe_relative("mods/sodium.jar").is_some());
        assert!(safe_relative("config/a/b.toml").is_some());
        for bad in [
            "../evil.jar",
            "/etc/passwd",
            "mods/../../x",
            "",
            "C:\\x.jar",
        ] {
            // Windows drive paths are only special on Windows; elsewhere they're a plain name.
            if bad.starts_with("C:") && !cfg!(windows) {
                continue;
            }
            assert!(safe_relative(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn only_trusted_hosts() {
        assert!(allowed_url("https://cdn.modrinth.com/data/x/y.jar"));
        assert!(allowed_url(
            "https://github.com/a/b/releases/download/1/x.jar"
        ));
        assert!(!allowed_url("http://cdn.modrinth.com/x.jar"));
        assert!(!allowed_url("https://cdn.modrinth.com.evil.example/x.jar"));
        assert!(!allowed_url("https://example.com/x.jar"));
    }

    #[test]
    fn reads_loader_from_dependencies() {
        let deps = HashMap::from([
            ("minecraft".to_string(), "1.21.1".to_string()),
            ("fabric-loader".to_string(), "0.16.5".to_string()),
        ]);
        assert_eq!(
            pack_loader(&deps),
            Some((LoaderKind::Fabric, "0.16.5".into()))
        );
    }
}
