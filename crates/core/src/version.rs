//! Version manifest and version JSON handling, including `inheritsFrom` merging for mod loaders.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::download::{self, Download};
use crate::error::{Result, other};
use crate::paths::{Paths, ensure_parent};
use crate::rules::Rule;

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const MOJANG_LIBRARIES: &str = "https://libraries.minecraft.net/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub sha1: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionJson {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(rename = "mainClass")]
    pub main_class: Option<String>,
    #[serde(rename = "inheritsFrom")]
    pub inherits_from: Option<String>,
    pub arguments: Option<Arguments>,
    /// Pre-1.13 space separated game arguments.
    #[serde(rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "assetIndex")]
    pub asset_index: Option<AssetIndexRef>,
    pub assets: Option<String>,
    pub downloads: Option<VersionDownloads>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(rename = "javaVersion")]
    pub java_version: Option<JavaVersion>,
    pub logging: Option<Logging>,
    /// Version whose `.jar` is the game jar. Filled in during resolution, not read from JSON.
    #[serde(skip)]
    pub jar: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
    /// Mojang's suggested heap / GC flags (newer versions only).
    #[serde(rename = "default-user-jvm", default)]
    pub default_user_jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional {
        #[serde(default)]
        rules: Vec<Rule>,
        value: ArgValue,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionDownloads {
    pub client: Option<Artifact>,
    /// The dedicated server jar (vanilla versions from 1.2.5 on).
    pub server: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    pub path: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaVersion {
    pub component: Option<String>,
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Logging {
    pub client: Option<LoggingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub argument: String,
    pub file: LoggingFile,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    /// Maven coordinate `group:artifact:version[:classifier][@ext]`.
    pub name: String,
    pub downloads: Option<LibraryDownloads>,
    /// Maven repository base, used by mod-loader libraries that have no `downloads`.
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Legacy natives: OS name -> classifier (may contain `${arch}`).
    pub natives: Option<HashMap<String, String>>,
    pub extract: Option<Extract>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDownloads {
    pub artifact: Option<Artifact>,
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Convert a Maven coordinate into a repository-relative path.
pub fn maven_path(name: &str, classifier_override: Option<&str>) -> Option<String> {
    let (coord, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let mut parts = coord.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    let classifier = classifier_override.or(parts.next());
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!(
        "{}/{artifact}/{version}/{file}",
        group.replace('.', "/")
    ))
}

impl Library {
    /// `group:artifact[:classifier]`, used to let a child version override a parent's library.
    fn identity(&self) -> String {
        let mut parts: Vec<&str> = self
            .name
            .split('@')
            .next()
            .unwrap_or("")
            .split(':')
            .collect();
        if parts.len() > 2 {
            parts.remove(2);
        }
        parts.join(":")
    }

    /// The main jar of this library, if it has one.
    pub fn artifact(&self) -> Option<Artifact> {
        if let Some(a) = self.downloads.as_ref().and_then(|d| d.artifact.clone()) {
            let path = a.path.clone().or_else(|| maven_path(&self.name, None));
            return Some(Artifact { path, ..a });
        }
        if self.downloads.is_some() {
            // Has a downloads block but no artifact: natives-only library.
            return None;
        }
        let path = maven_path(&self.name, None)?;
        let base = self.url.as_deref().unwrap_or(MOJANG_LIBRARIES);
        Some(Artifact {
            url: format!("{}/{path}", base.trim_end_matches('/')),
            path: Some(path),
            sha1: self.sha1.clone(),
            size: self.size,
        })
    }

    /// Legacy natives jar for the given OS, if this library has one.
    pub fn native_artifact(&self, os_name: &str, arch_bits: &str) -> Option<Artifact> {
        let classifier = self
            .natives
            .as_ref()?
            .get(os_name)?
            .replace("${arch}", arch_bits);
        if let Some(a) = self
            .downloads
            .as_ref()
            .and_then(|d| d.classifiers.as_ref())
            .and_then(|c| c.get(&classifier))
        {
            let path = a
                .path
                .clone()
                .or_else(|| maven_path(&self.name, Some(&classifier)));
            return Some(Artifact { path, ..a.clone() });
        }
        let path = maven_path(&self.name, Some(&classifier))?;
        let base = self.url.as_deref().unwrap_or(MOJANG_LIBRARIES);
        Some(Artifact {
            url: format!("{}/{path}", base.trim_end_matches('/')),
            path: Some(path),
            sha1: None,
            size: None,
        })
    }
}

/// Fetch the version manifest, falling back to the cached copy when offline.
pub async fn manifest(http: &reqwest::Client, paths: &Paths) -> Result<VersionManifest> {
    let cache = paths.meta().join("version_manifest_v2.json");
    match download::get_json::<VersionManifest>(http, MANIFEST_URL).await {
        Ok(m) => {
            ensure_parent(&cache)?;
            std::fs::write(&cache, serde_json::to_vec(&m)?)?;
            Ok(m)
        }
        Err(e) => match std::fs::read(&cache) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(_) => Err(e),
        },
    }
}

/// Load a version JSON from disk, downloading vanilla ones from Mojang when missing.
async fn load_one(http: &reqwest::Client, paths: &Paths, id: &str) -> Result<VersionJson> {
    let file = paths.version_json(id);
    if !file.exists() {
        let manifest = manifest(http, paths).await?;
        let entry = manifest
            .versions
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| other(format!("unknown version '{id}'")))?;
        download::fetch(
            http,
            &Download {
                url: entry.url.clone(),
                dest: file.clone(),
                sha1: Some(entry.sha1.clone()),
                size: None,
            },
        )
        .await?;
    }
    Ok(serde_json::from_slice(&std::fs::read(&file)?)?)
}

/// Load a version and merge its whole `inheritsFrom` chain into a single JSON.
pub async fn resolve(http: &reqwest::Client, paths: &Paths, id: &str) -> Result<VersionJson> {
    let mut chain = vec![load_one(http, paths, id).await?];
    let mut seen = HashSet::from([id.to_string()]);
    while let Some(parent) = chain.last().unwrap().inherits_from.clone() {
        if !seen.insert(parent.clone()) {
            return Err(other(format!("inheritsFrom cycle at '{parent}'")));
        }
        chain.push(load_one(http, paths, &parent).await?);
    }

    let mut merged = chain.pop().unwrap();
    merged.jar = merged.id.clone();
    while let Some(child) = chain.pop() {
        merged = merge(merged, child);
    }
    if merged.main_class.is_none() {
        return Err(other(format!("version '{id}' has no mainClass")));
    }
    Ok(merged)
}

fn merge(parent: VersionJson, child: VersionJson) -> VersionJson {
    // Child libraries come first and replace parent libraries with the same identity.
    let child_ids: HashSet<String> = child.libraries.iter().map(Library::identity).collect();
    let mut libraries = child.libraries;
    libraries.extend(
        parent
            .libraries
            .into_iter()
            .filter(|l| !child_ids.contains(&l.identity())),
    );

    let arguments = match (parent.arguments, child.arguments) {
        (Some(mut p), Some(c)) => {
            p.game.extend(c.game);
            p.jvm.extend(c.jvm);
            if !c.default_user_jvm.is_empty() {
                p.default_user_jvm = c.default_user_jvm;
            }
            Some(p)
        }
        (p, c) => c.or(p),
    };

    // A child that ships its own client download (rare) also brings its own jar.
    let jar = if child.downloads.as_ref().is_some_and(|d| d.client.is_some()) {
        child.id.clone()
    } else {
        parent.jar
    };

    VersionJson {
        id: child.id,
        kind: if child.kind.is_empty() {
            parent.kind
        } else {
            child.kind
        },
        main_class: child.main_class.or(parent.main_class),
        inherits_from: None,
        arguments,
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        downloads: child.downloads.or(parent.downloads),
        libraries,
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
        jar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.4.3:natives-linux", None).unwrap(),
            "org/lwjgl/lwjgl/3.4.3/lwjgl-3.4.3-natives-linux.jar"
        );
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.16.0", None).unwrap(),
            "net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar"
        );
        assert_eq!(
            maven_path("a.b:c:1.0@zip", Some("x")).unwrap(),
            "a/b/c/1.0/c-1.0-x.zip"
        );
    }

    #[test]
    fn child_library_overrides_parent() {
        let parent: VersionJson = serde_json::from_str(
            r#"{"id":"1.21","mainClass":"a.Main","libraries":[{"name":"org.ow2.asm:asm:9.1"},{"name":"x:y:1"}]}"#,
        )
        .unwrap();
        let child: VersionJson = serde_json::from_str(
            r#"{"id":"fabric","inheritsFrom":"1.21","mainClass":"b.Main","libraries":[{"name":"org.ow2.asm:asm:9.7"}]}"#,
        )
        .unwrap();
        let mut parent = parent;
        parent.jar = parent.id.clone();
        let m = merge(parent, child);
        let names: Vec<_> = m.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["org.ow2.asm:asm:9.7", "x:y:1"]);
        assert_eq!(m.main_class.as_deref(), Some("b.Main"));
        assert_eq!(m.jar, "1.21");
    }
}
