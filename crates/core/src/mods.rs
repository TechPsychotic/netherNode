//! Mods: the jars in an instance's `mods/` folder, and installing from Modrinth.
//!
//! What we installed from Modrinth is remembered in `<instance>/mods.json` (file name ->
//! project), so the list can show real names and dependencies aren't installed twice.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{self, Download, Progress, ProgressFn};
use crate::error::{Result, other};
use crate::instance::Instance;
use crate::loader::LoaderKind;
use crate::paths::Paths;

const MODRINTH: &str = "https://api.modrinth.com/v2";
const DISABLED: &str = ".disabled";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub version_number: String,
}

/// `mods.json`: enabled file name (without `.disabled`) -> what it is.
type Index = BTreeMap<String, IndexEntry>;

fn index_path(paths: &Paths, instance: &Instance) -> PathBuf {
    instance.dir(paths).join("mods.json")
}

fn load_index(paths: &Paths, instance: &Instance) -> Index {
    std::fs::read(index_path(paths, instance))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_index(paths: &Paths, instance: &Instance, index: &Index) -> Result<()> {
    std::fs::write(
        index_path(paths, instance),
        serde_json::to_vec_pretty(index)?,
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledMod {
    /// File name without `.disabled`.
    pub file_name: String,
    pub enabled: bool,
    /// Known when installed from Modrinth.
    pub info: Option<IndexEntry>,
}

impl InstalledMod {
    pub fn display_name(&self) -> &str {
        self.info
            .as_ref()
            .map_or(self.file_name.trim_end_matches(".jar"), |i| &i.title)
    }
}

pub fn list(paths: &Paths, instance: &Instance) -> Result<Vec<InstalledMod>> {
    let index = load_index(paths, instance);
    let mut mods = Vec::new();
    let Ok(entries) = std::fs::read_dir(instance.mods_dir(paths)) else {
        return Ok(mods);
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        let (file_name, enabled) = match name.strip_suffix(DISABLED) {
            Some(base) => (base.to_string(), false),
            None => (name, true),
        };
        if !file_name.ends_with(".jar") {
            continue;
        }
        mods.push(InstalledMod {
            info: index.get(&file_name).cloned(),
            file_name,
            enabled,
        });
    }
    mods.sort_by_key(|m| m.display_name().to_lowercase());
    Ok(mods)
}

fn mod_path(paths: &Paths, instance: &Instance, file_name: &str, enabled: bool) -> PathBuf {
    let name = if enabled {
        file_name.to_string()
    } else {
        format!("{file_name}{DISABLED}")
    };
    instance.mods_dir(paths).join(name)
}

/// Turn a mod on or off by renaming it to/from `<name>.jar.disabled`, which every loader skips.
pub fn set_enabled(
    paths: &Paths,
    instance: &Instance,
    file_name: &str,
    enabled: bool,
) -> Result<()> {
    let from = mod_path(paths, instance, file_name, !enabled);
    if from.exists() {
        std::fs::rename(from, mod_path(paths, instance, file_name, enabled))?;
    }
    Ok(())
}

pub fn remove(paths: &Paths, instance: &Instance, file_name: &str) -> Result<()> {
    for enabled in [true, false] {
        let path = mod_path(paths, instance, file_name, enabled);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    let mut index = load_index(paths, instance);
    if index.remove(file_name).is_some() {
        save_index(paths, instance, &index)?;
    }
    Ok(())
}

/// Copy a jar the user picked into the mods folder.
pub fn add_file(paths: &Paths, instance: &Instance, source: &Path) -> Result<String> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.ends_with(".jar"))
        .ok_or_else(|| other("mods must be .jar files"))?
        .to_string();
    let dir = instance.mods_dir(paths);
    std::fs::create_dir_all(&dir)?;
    std::fs::copy(source, dir.join(&name))?;
    Ok(name)
}

fn mod_loaders(instance: &Instance) -> Result<&'static [&'static str]> {
    let loaders = instance
        .loader
        .as_ref()
        .map_or(&[][..], |l| l.kind.mod_loaders());
    if loaders.is_empty() {
        let why = match instance.loader.as_ref().map(|l| l.kind) {
            Some(LoaderKind::OptiFine) => "OptiFine instances can't load mods",
            _ => "this instance has no mod loader",
        };
        return Err(other(format!(
            "{why}; install Fabric, Quilt, Forge or NeoForge first"
        )));
    }
    Ok(loaders)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total_hits: u64,
}

/// Search Modrinth for mods that work on this instance's loader and Minecraft version.
pub async fn search(
    http: &reqwest::Client,
    instance: &Instance,
    query: &str,
    offset: u32,
) -> Result<SearchResults> {
    let loaders = mod_loaders(instance)?;
    let facets = serde_json::json!([
        ["project_type:mod"],
        loaders
            .iter()
            .map(|l| format!("categories:{l}"))
            .collect::<Vec<_>>(),
        [format!("versions:{}", instance.minecraft_version())],
    ]);
    let index = if query.trim().is_empty() {
        "downloads"
    } else {
        "relevance"
    };
    let resp = http
        .get(format!("{MODRINTH}/search"))
        .query(&[
            ("query", query),
            ("facets", &facets.to_string()),
            ("index", index),
            ("limit", "20"),
            ("offset", &offset.to_string()),
        ])
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

#[derive(Debug, Deserialize)]
struct Project {
    id: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct Version {
    id: String,
    project_id: String,
    version_number: String,
    /// release, beta or alpha.
    #[serde(default)]
    version_type: String,
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
    hashes: std::collections::HashMap<String, String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
    project_id: Option<String>,
    version_id: Option<String>,
    dependency_type: String,
}

/// Install a Modrinth project (id or slug) and every required dependency, newest compatible
/// versions. Returns the titles that were newly installed.
pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    instance: &Instance,
    project: &str,
    progress: &ProgressFn,
) -> Result<Vec<String>> {
    let loaders = mod_loaders(instance)?;
    let mc = instance.minecraft_version();
    let mut index = load_index(paths, instance);
    let mut have: HashSet<String> = index.values().map(|e| e.project_id.clone()).collect();
    let mut queue = VecDeque::from([(project.to_string(), true)]);
    let mut installed = Vec::new();
    std::fs::create_dir_all(instance.mods_dir(paths))?;

    while let Some((wanted, top_level)) = queue.pop_front() {
        let info: Project =
            download::get_json(http, &format!("{MODRINTH}/project/{wanted}")).await?;
        if !have.insert(info.id.clone()) {
            continue;
        }
        let versions: Vec<Version> = http
            .get(format!("{MODRINTH}/project/{}/version", info.id))
            .query(&[
                ("loaders", serde_json::to_string(loaders)?),
                ("game_versions", serde_json::to_string(&[mc])?),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        // Newest stable release, or the newest of any kind if there is no release.
        let pick = versions
            .iter()
            .position(|v| v.version_type == "release")
            .unwrap_or(0);
        let Some(version) = versions.into_iter().nth(pick) else {
            let msg = format!(
                "{} has no version for Minecraft {mc} with this loader",
                info.title
            );
            if top_level {
                return Err(other(msg));
            }
            eprintln!("warning: {msg}");
            continue;
        };
        let file = version
            .files
            .iter()
            .find(|f| f.primary)
            .or(version.files.first())
            .ok_or_else(|| {
                other(format!(
                    "{} {} has no files",
                    info.title, version.version_number
                ))
            })?;

        progress(Progress::Stage {
            name: info.title.clone(),
            total: 1,
        });
        let dest = instance.mods_dir(paths).join(&file.filename);
        download::fetch(
            http,
            &Download {
                url: file.url.clone(),
                dest,
                sha1: file.hashes.get("sha1").cloned(),
                size: file.size,
            },
        )
        .await?;
        progress(Progress::Advance);

        index.insert(
            file.filename.clone(),
            IndexEntry {
                project_id: version.project_id.clone(),
                version_id: version.id.clone(),
                title: info.title.clone(),
                version_number: version.version_number.clone(),
            },
        );
        save_index(paths, instance, &index)?;
        installed.push(info.title);

        for dep in version
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == "required")
        {
            let project = match (&dep.project_id, &dep.version_id) {
                (Some(p), _) => p.clone(),
                (None, Some(v)) => {
                    let version: Version =
                        download::get_json(http, &format!("{MODRINTH}/version/{v}")).await?;
                    version.project_id
                }
                (None, None) => continue,
            };
            queue.push_back((project, false));
        }
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_instance(name: &str) -> (Paths, Instance) {
        let root = std::env::temp_dir().join(format!(
            "nethernode-mods-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::new(root);
        let instance = Instance::create(&paths, "test", "1.21.1", false).unwrap();
        (paths, instance)
    }

    #[test]
    fn list_toggle_remove() {
        let (paths, instance) = temp_instance("toggle");
        let dir = instance.mods_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sodium.jar"), b"x").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();

        let mods = list(&paths, &instance).unwrap();
        assert_eq!(mods.len(), 1);
        assert!(mods[0].enabled);

        set_enabled(&paths, &instance, "sodium.jar", false).unwrap();
        assert!(dir.join("sodium.jar.disabled").exists());
        assert!(!list(&paths, &instance).unwrap()[0].enabled);

        remove(&paths, &instance, "sodium.jar").unwrap();
        assert!(list(&paths, &instance).unwrap().is_empty());
        std::fs::remove_dir_all(&paths.root).unwrap();
    }

    #[test]
    fn vanilla_has_no_mods() {
        let (paths, instance) = temp_instance("vanilla");
        assert!(mod_loaders(&instance).is_err());
        std::fs::remove_dir_all(&paths.root).unwrap();
    }
}
