//! Finding a Java runtime: Mojang's bundled runtimes first, then the system `java`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::download::{self, Download, ProgressFn};
use crate::error::{Result, other};
use crate::paths::Paths;
use crate::rules::Environment;
use crate::version::JavaVersion;

const RUNTIME_INDEX_URL: &str = "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Debug, Deserialize)]
struct RuntimeEntry {
    manifest: ManifestRef,
}

#[derive(Debug, Deserialize)]
struct ManifestRef {
    sha1: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, RuntimeFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RuntimeFile {
    Directory,
    File {
        downloads: RuntimeDownloads,
        #[serde(default)]
        executable: bool,
    },
    Link {
        #[cfg_attr(not(unix), allow(dead_code))]
        target: String,
    },
}

#[derive(Debug, Deserialize)]
struct RuntimeDownloads {
    raw: RuntimeRaw,
}

#[derive(Debug, Deserialize)]
struct RuntimeRaw {
    sha1: String,
    size: u64,
    url: String,
}

/// A Java executable plus the CPU architecture it runs as, which decides which natives load.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub java: PathBuf,
    /// Mojang arch name (`x86_64`, `arm64`, `x86`), or `None` for "same as the host".
    pub arch: Option<&'static str>,
}

/// Runtime index keys to try, in order, with the arch each one runs as. ARM machines fall back
/// to x86-64 runtimes, which run under Rosetta 2 (macOS) or Windows' x64 emulation. Mojang
/// publishes no ARM builds of Java 8 or 16, so Minecraft 1.17 and older need this.
fn platform_candidates(env: &Environment) -> &'static [(&'static str, &'static str)] {
    match (env.os_name, env.arch) {
        ("linux", "x86_64") => &[("linux", "x86_64")],
        ("linux", "x86") => &[("linux-i386", "x86")],
        ("osx", "x86_64") => &[("mac-os", "x86_64")],
        ("osx", "arm64") => &[("mac-os-arm64", "arm64"), ("mac-os", "x86_64")],
        ("windows", "x86_64") => &[("windows-x64", "x86_64")],
        ("windows", "x86") => &[("windows-x86", "x86")],
        ("windows", "arm64") => &[("windows-arm64", "arm64"), ("windows-x64", "x86_64")],
        _ => &[],
    }
}

fn java_binary(home: &Path, env: &Environment) -> PathBuf {
    let home = if env.os_name == "osx" {
        home.join("jre.bundle/Contents/Home")
    } else {
        home.to_path_buf()
    };
    home.join("bin").join(if env.os_name == "windows" {
        "java.exe"
    } else {
        "java"
    })
}

/// Pick a Java for `required`. Order: Mojang runtime (downloaded on demand, native first,
/// then emulated), then `$JAVA_HOME`, then `java` on the PATH, if its major version matches.
pub async fn resolve(
    http: &reqwest::Client,
    paths: &Paths,
    env: &Environment,
    required: &JavaVersion,
    progress: &ProgressFn,
) -> Result<Runtime> {
    let component = required.component.as_deref().unwrap_or("jre-legacy");
    match install_mojang_runtime(http, paths, env, component, progress).await {
        Ok(runtime) => return Ok(runtime),
        Err(e) => eprintln!(
            "warning: Mojang Java runtime '{component}' unavailable ({e}), looking for an installed Java"
        ),
    }

    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        candidates.push(
            Path::new(&home)
                .join("bin")
                .join(if env.os_name == "windows" {
                    "java.exe"
                } else {
                    "java"
                }),
        );
    }
    candidates.push(PathBuf::from("java"));
    let mut found = Vec::new();
    for java in candidates {
        match major_version(&java) {
            Some(v) if v == required.major_version => return Ok(Runtime { java, arch: None }),
            Some(v) => found.push(format!("{} is Java {v}", java.display())),
            None => {}
        }
    }
    Err(other(format!(
        "this version needs Java {} and none was found{}; install it or set java_path on the instance",
        required.major_version,
        if found.is_empty() {
            String::new()
        } else {
            format!(" ({})", found.join(", "))
        }
    )))
}

type RuntimeIndex = HashMap<String, HashMap<String, Vec<RuntimeEntry>>>;

async fn install_mojang_runtime(
    http: &reqwest::Client,
    paths: &Paths,
    env: &Environment,
    component: &str,
    progress: &ProgressFn,
) -> Result<Runtime> {
    let candidates = platform_candidates(env);
    if candidates.is_empty() {
        return Err(other("Mojang publishes no Java runtimes for this platform"));
    }
    let home_for = |platform: &str| paths.runtimes().join(platform).join(component);

    let index: RuntimeIndex = match download::get_json(http, RUNTIME_INDEX_URL).await {
        Ok(i) => i,
        Err(e) => {
            // Offline: use whatever matching runtime is already installed.
            for (platform, arch) in candidates {
                let java = java_binary(&home_for(platform), env);
                if java.exists() {
                    return Ok(Runtime {
                        java,
                        arch: Some(arch),
                    });
                }
            }
            return Err(e);
        }
    };

    let (platform, arch, entry) = candidates
        .iter()
        .find_map(|(platform, arch)| {
            let entry = index.get(*platform)?.get(component)?.first()?;
            Some((*platform, *arch, entry))
        })
        .ok_or_else(|| {
            other(format!(
                "runtime '{component}' is not published for this platform"
            ))
        })?;
    if arch != env.arch {
        eprintln!(
            "note: no native {component} for {}-{}, using the {arch} build under emulation",
            env.os_name, env.arch
        );
    }

    let home = home_for(platform);
    let marker = home.join(".nethernode-sha1");
    let java = java_binary(&home, env);
    let runtime = Runtime {
        java: java.clone(),
        arch: Some(arch),
    };
    if java.exists()
        && std::fs::read_to_string(&marker).is_ok_and(|s| s.trim() == entry.manifest.sha1)
    {
        return Ok(runtime);
    }

    let manifest: RuntimeManifest = download::get_json(http, &entry.manifest.url).await?;
    let mut downloads = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (rel, file) in &manifest.files {
        let dest = home.join(rel);
        match file {
            RuntimeFile::Directory => std::fs::create_dir_all(&dest)?,
            RuntimeFile::File {
                downloads: d,
                executable,
            } => {
                if *executable {
                    executables.push(dest.clone());
                }
                downloads.push(Download {
                    url: d.raw.url.clone(),
                    dest,
                    sha1: Some(d.raw.sha1.clone()),
                    size: Some(d.raw.size),
                });
            }
            RuntimeFile::Link { target } => links.push((dest, target.clone())),
        }
    }
    download::fetch_all(
        http,
        &format!("Java runtime ({component})"),
        downloads,
        progress,
    )
    .await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in executables {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
        for (link, target) in links {
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(target, link)?;
        }
    }
    // Windows runtimes contain no symlinks, and files need no executable bit.
    #[cfg(not(unix))]
    let _ = (executables, links);

    std::fs::write(&marker, &entry.manifest.sha1)?;
    Ok(runtime)
}

/// Run `java -version` and parse the major version (`1.8.0_x` -> 8, `21.0.2` -> 21).
pub fn major_version(java: &Path) -> Option<u32> {
    let out = std::process::Command::new(java)
        .arg("-version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let quoted = text.split('"').nth(1)?;
    parse_major(quoted)
}

fn parse_major(version: &str) -> Option<u32> {
    let mut parts = version.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_major;

    #[test]
    fn parses_java_versions() {
        assert_eq!(parse_major("1.8.0_402"), Some(8));
        assert_eq!(parse_major("21.0.2"), Some(21));
        assert_eq!(parse_major("25"), Some(25));
        assert_eq!(parse_major("17-ea"), Some(17));
    }
}
