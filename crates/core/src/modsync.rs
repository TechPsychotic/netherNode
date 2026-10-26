//! Matching mods between friends. A modded world or server only accepts players with the
//! same loader and mods, so the host describes what it runs (a [`Manifest`]) and the friend's
//! netherNode recreates it: mods Modrinth knows come from Modrinth, anything else straight
//! from the host over the encrypted tunnel. Every file is checked against the host's SHA-1.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::download::{self, ProgressFn};
use crate::error::{Result, other};
use crate::instance::Instance;
use crate::loader::{self, Loader, LoaderKind};
use crate::paths::Paths;

/// Largest mod jar accepted from a host.
pub const MAX_MOD_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub minecraft: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<Loader>,
    pub mods: Vec<ModFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModFile {
    pub file_name: String,
    pub sha1: String,
    pub size: u64,
    /// Where Modrinth serves this exact file, if it's on Modrinth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Manifest {
    /// Whether joining needs anything beyond plain Minecraft.
    pub fn is_modded(&self) -> bool {
        self.loader.is_some() || !self.mods.is_empty()
    }

    /// e.g. "Fabric 0.16.5 · 42 mods".
    pub fn summary(&self) -> String {
        let loader = self.loader.as_ref().map_or("Vanilla".to_string(), |l| {
            format!("{} {}", l.kind, l.version)
        });
        format!(
            "Minecraft {} · {loader} · {} mods",
            self.minecraft,
            self.mods.len()
        )
    }
}

/// The enabled mod jars in `mods_dir`.
fn jars(mods_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<_> = std::fs::read_dir(mods_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jar") && p.is_file())
        .collect();
    out.sort();
    out
}

/// Describe the mods in `mods_dir` for friends. Modrinth links are looked up by hash;
/// without internet the files are still listed (friends then fetch them from the host).
pub async fn build(
    http: &reqwest::Client,
    minecraft: &str,
    loader: Option<&Loader>,
    mods_dir: &Path,
) -> Result<Manifest> {
    let mut mods = Vec::new();
    for jar in jars(mods_dir) {
        let Some(file_name) = jar.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        mods.push(ModFile {
            sha1: download::sha1_file(&jar)?,
            size: jar.metadata()?.len(),
            file_name,
            url: None,
        });
    }
    let hashes: Vec<String> = mods.iter().map(|m| m.sha1.clone()).collect();
    if let Ok(known) = crate::mods::known_files(http, &hashes).await {
        for m in &mut mods {
            m.url = known.get(&m.sha1).map(|k| k.url.clone());
        }
    }
    Ok(Manifest {
        minecraft: minecraft.to_string(),
        // OptiFine isn't something a friend's netherNode can install for them.
        loader: loader.filter(|l| l.kind != LoaderKind::OptiFine).cloned(),
        mods,
    })
}

/// The path of a jar in `mods_dir` with this SHA-1, for serving it to a friend. Only jars
/// listed in `manifest` are ever served.
pub fn file_for(manifest: &Manifest, mods_dir: &Path, sha1: &str) -> Option<std::path::PathBuf> {
    let entry = manifest.mods.iter().find(|m| m.sha1 == sha1)?;
    let path = mods_dir.join(&entry.file_name);
    (download::sha1_file(&path).ok()? == sha1).then_some(path)
}

/// An instance that already matches: same Minecraft, same loader, and every mod present.
pub fn find_matching(paths: &Paths, manifest: &Manifest) -> Option<Instance> {
    Instance::list(paths).ok()?.into_iter().find(|i| {
        let same_loader = match (&i.loader, &manifest.loader) {
            (None, None) => true,
            (Some(a), Some(b)) => a.kind == b.kind && a.version == b.version,
            _ => false,
        };
        if i.minecraft_version() != manifest.minecraft || !same_loader {
            return false;
        }
        let have: HashSet<String> = jars(&i.mods_dir(paths))
            .iter()
            .filter_map(|j| download::sha1_file(j).ok())
            .collect();
        manifest.mods.iter().all(|m| have.contains(&m.sha1))
    })
}

/// Create instance `name` matching `manifest`. `fetch` gets a mod from the host by SHA-1,
/// for mods that aren't on Modrinth.
pub async fn create_instance<F, Fut>(
    http: &reqwest::Client,
    paths: &Paths,
    manifest: &Manifest,
    name: &str,
    fetch: F,
    progress: &ProgressFn,
) -> Result<Instance>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>>>,
{
    let low_end = crate::memory::SystemMemory::current().total_mb <= 8 * 1024;
    let mut instance = Instance::create(paths, name, &manifest.minecraft, low_end)?;
    let result = async {
        if let Some(l) = &manifest.loader {
            loader::apply(
                http,
                paths,
                &mut instance,
                l.kind,
                Some(&l.version),
                progress,
            )
            .await?;
        }
        let mods_dir = instance.mods_dir(paths);
        std::fs::create_dir_all(&mods_dir)?;
        progress(download::Progress::Stage {
            name: "Mods".into(),
            total: manifest.mods.len() as u64,
        });
        for m in &manifest.mods {
            let bytes = match &m.url {
                Some(url) => match fetch_url(http, url, m).await {
                    Ok(bytes) => bytes,
                    Err(_) => fetch(m.sha1.clone()).await?,
                },
                None => fetch(m.sha1.clone()).await?,
            };
            if !hash_matches(&bytes, &m.sha1) {
                return Err(other(format!(
                    "{} didn't match the host's copy",
                    m.file_name
                )));
            }
            // Never let a file name escape the mods folder.
            let file = Path::new(&m.file_name)
                .file_name()
                .ok_or_else(|| other("bad mod file name"))?;
            std::fs::write(mods_dir.join(file), bytes)?;
            progress(download::Progress::Advance);
        }
        Ok::<(), crate::Error>(())
    }
    .await;
    if let Err(e) = result {
        let _ = Instance::delete(paths, &instance.name);
        return Err(e);
    }
    Ok(instance)
}

async fn fetch_url(http: &reqwest::Client, url: &str, m: &ModFile) -> Result<Vec<u8>> {
    if m.size > MAX_MOD_SIZE {
        return Err(other("mod too large"));
    }
    Ok(http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

fn hash_matches(bytes: &[u8], sha1: &str) -> bool {
    use sha1::{Digest, Sha1};
    hex::encode(Sha1::digest(bytes)) == sha1
}

/// Whether a Fabric/Quilt jar says it only runs on the client (`"environment": "client"`).
pub(crate) fn fabric_client_only(jar: &Path) -> bool {
    let Ok(file) = std::fs::File::open(jar) else {
        return false;
    };
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return false;
    };
    let mut text = String::new();
    let Ok(mut entry) = zip.by_name("fabric.mod.json") else {
        return false;
    };
    if entry.read_to_string(&mut text).is_err() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("environment")
                .and_then(|e| e.as_str())
                .map(|e| e == "client")
        })
        .unwrap_or(false)
}

/// Copy an instance's mods (and their configs) to a server, leaving out client-only mods
/// that would stop a dedicated server from starting. Returns the names of mods left out.
pub async fn copy_to_server(
    http: &reqwest::Client,
    instance_game_dir: &Path,
    server_dir: &Path,
) -> Result<Vec<String>> {
    let from = instance_game_dir.join("mods");
    let to = server_dir.join("mods");
    std::fs::create_dir_all(&to)?;
    let files = jars(&from);
    let hashes: Vec<(std::path::PathBuf, String)> = files
        .iter()
        .filter_map(|j| Some((j.clone(), download::sha1_file(j).ok()?)))
        .collect();
    let all: Vec<String> = hashes.iter().map(|(_, h)| h.clone()).collect();
    let known = crate::mods::known_files(http, &all)
        .await
        .unwrap_or_default();
    let ids: Vec<String> = known.values().map(|k| k.project_id.clone()).collect();
    let client_only = crate::mods::client_only_projects(http, &ids)
        .await
        .unwrap_or_default();
    let mut skipped = Vec::new();
    for (jar, hash) in hashes {
        let name = jar
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let modrinth_says_client = known
            .get(&hash)
            .is_some_and(|k| client_only.contains(&k.project_id));
        if modrinth_says_client || fabric_client_only(&jar) {
            skipped.push(name);
            continue;
        }
        std::fs::copy(&jar, to.join(&name))?;
    }
    // Mods read their settings from config/; bring the instance's along.
    for dir in ["config", "defaultconfigs"] {
        let source = instance_game_dir.join(dir);
        if source.is_dir() {
            copy_dir(&source, &server_dir.join(dir))?;
        }
    }
    Ok(skipped)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_summary_and_lookup() {
        let dir = std::env::temp_dir().join(format!("nethernode-modsync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jar"), b"mod a").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a mod").unwrap();
        let sha1 = download::sha1_file(&dir.join("a.jar")).unwrap();
        let manifest = Manifest {
            minecraft: "1.21.1".into(),
            loader: Some(Loader {
                kind: LoaderKind::Fabric,
                version: "0.16.5".into(),
            }),
            mods: vec![ModFile {
                file_name: "a.jar".into(),
                sha1: sha1.clone(),
                size: 5,
                url: None,
            }],
        };
        assert!(manifest.is_modded());
        assert_eq!(
            manifest.summary(),
            "Minecraft 1.21.1 · Fabric 0.16.5 · 1 mods"
        );
        assert_eq!(file_for(&manifest, &dir, &sha1), Some(dir.join("a.jar")));
        // Unlisted files are never served, even if they exist.
        let txt = download::sha1_file(&dir.join("notes.txt")).unwrap();
        assert_eq!(file_for(&manifest, &dir, &txt), None);
        assert!(hash_matches(b"mod a", &sha1));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
