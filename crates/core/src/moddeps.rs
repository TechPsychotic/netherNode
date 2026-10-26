//! Missing mod dependencies. Every mod jar says which other mods it needs (Fabric's
//! `fabric.mod.json`, Quilt's `quilt.mod.json`, Forge's `mods.toml`, NeoForge's
//! `neoforge.mods.toml`). Before a modded instance starts, anything required but absent is
//! looked up on Modrinth and installed, instead of the game stopping with "requires mod X,
//! which is missing".

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use crate::download::ProgressFn;
use crate::error::Result;
use crate::instance::Instance;
use crate::paths::Paths;

/// A required mod that isn't installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    /// The mod id the jars ask for (e.g. `geckolib`).
    pub id: String,
    /// Names of the mods that need it.
    pub needed_by: Vec<String>,
}

/// Ids the loader or game provide themselves.
const BUILT_IN: &[&str] = &[
    "minecraft",
    "java",
    "fabricloader",
    "fabric-loader",
    "quilt_loader",
    "quilt_base",
    "forge",
    "neoforge",
    "fml",
    "javafml",
    "lowcodefml",
    "mixinextras",
];

#[derive(Default)]
struct JarInfo {
    name: String,
    provides: Vec<String>,
    requires: Vec<String>,
}

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut entry = zip.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn fabric(json: &serde_json::Value, info: &mut JarInfo) {
    if let Some(id) = json["id"].as_str() {
        info.provides.push(id.to_string());
        if info.name.is_empty() {
            info.name = json["name"].as_str().unwrap_or(id).to_string();
        }
    }
    if let Some(provides) = json["provides"].as_array() {
        info.provides
            .extend(provides.iter().filter_map(|p| p.as_str().map(String::from)));
    }
    if let Some(depends) = json["depends"].as_object() {
        info.requires.extend(depends.keys().cloned());
    }
}

fn quilt(json: &serde_json::Value, info: &mut JarInfo) {
    let loader = &json["quilt_loader"];
    if let Some(id) = loader["id"].as_str() {
        info.provides.push(id.to_string());
        if info.name.is_empty() {
            info.name = loader["metadata"]["name"]
                .as_str()
                .unwrap_or(id)
                .to_string();
        }
    }
    for dep in loader["depends"].as_array().into_iter().flatten() {
        match dep {
            serde_json::Value::String(id) => info.requires.push(id.clone()),
            serde_json::Value::Object(o)
                if !o.get("optional").and_then(|v| v.as_bool()).unwrap_or(false) =>
            {
                if let Some(id) = o.get("id").and_then(|v| v.as_str()) {
                    // "group:id" in Quilt; mods are matched by id.
                    info.requires
                        .push(id.rsplit(':').next().unwrap_or(id).to_string());
                }
            }
            _ => {}
        }
    }
}

fn forge(text: &str, info: &mut JarInfo) {
    let Ok(toml) = text.parse::<toml::Table>() else {
        return;
    };
    for m in toml
        .get("mods")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(id) = m.get("modId").and_then(|v| v.as_str()) {
            info.provides.push(id.to_string());
            if info.name.is_empty() {
                info.name = m
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(id)
                    .to_string();
            }
        }
    }
    let Some(deps) = toml.get("dependencies").and_then(|d| d.as_table()) else {
        return;
    };
    for list in deps.values().filter_map(|v| v.as_array()) {
        for dep in list.iter().filter_map(|d| d.as_table()) {
            // Forge: mandatory = true. NeoForge: type = "required" (older ones: mandatory).
            let required = dep
                .get("mandatory")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || dep
                    .get("type")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t.eq_ignore_ascii_case("required"));
            // Mods needed only on the server side don't matter for the game.
            let server_only = dep
                .get("side")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("SERVER"));
            if required
                && !server_only
                && let Some(id) = dep.get("modId").and_then(|v| v.as_str())
            {
                info.requires.push(id.to_string());
            }
        }
    }
}

/// What one jar (and the jars nested inside it) provides and requires.
fn inspect<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, nested: bool, info: &mut JarInfo) {
    let mut own = JarInfo::default();
    if let Some(bytes) = read_entry(zip, "fabric.mod.json")
        && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes)
    {
        fabric(&json, &mut own);
    }
    if let Some(bytes) = read_entry(zip, "quilt.mod.json")
        && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes)
    {
        quilt(&json, &mut own);
    }
    for name in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(bytes) = read_entry(zip, name) {
            forge(&String::from_utf8_lossy(&bytes), &mut own);
        }
    }
    info.provides.extend(own.provides);
    if nested {
        // Bundled libraries satisfy others' needs; their own needs are the outer mod's job.
        return;
    }
    info.name = own.name;
    info.requires.extend(own.requires);
    // Jars bundled inside (Fabric's META-INF/jars, Forge's META-INF/jarjar).
    let inner: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| n.starts_with("META-INF/") && n.ends_with(".jar"))
        .collect();
    for name in inner {
        if let Some(bytes) = read_entry(zip, &name)
            && let Ok(mut nested_zip) = zip::ZipArchive::new(Cursor::new(bytes))
        {
            inspect(&mut nested_zip, true, info);
        }
    }
}

/// Required mod ids missing from `mods_dir`, with who needs each.
pub fn missing(mods_dir: &Path) -> Vec<Missing> {
    let mut provided: HashSet<String> = BUILT_IN.iter().map(|s| s.to_string()).collect();
    let mut needs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let jars = std::fs::read_dir(mods_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jar"));
    for jar in jars {
        let Ok(file) = std::fs::File::open(&jar) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(file) else {
            continue;
        };
        let mut info = JarInfo::default();
        inspect(&mut zip, false, &mut info);
        if info.name.is_empty() {
            info.name = jar
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
        }
        provided.extend(info.provides);
        for id in info.requires {
            needs.entry(id).or_default().push(info.name.clone());
        }
    }
    needs
        .into_iter()
        .filter(|(id, _)| !provided.contains(id))
        .map(|(id, mut needed_by)| {
            needed_by.sort();
            needed_by.dedup();
            Missing { id, needed_by }
        })
        .collect()
}

/// Modrinth projects that might be the mod with this id: its slug spelled a few ways, then
/// a search for it among mods for this loader and version.
async fn candidates(http: &reqwest::Client, instance: &Instance, id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for slug in [
        id.to_string(),
        id.replace('_', "-"),
        format!("{}-api", id.replace('_', "-")),
    ] {
        if out.contains(&slug) {
            continue;
        }
        let found = http
            .get(format!("https://api.modrinth.com/v2/project/{slug}"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if found {
            out.push(slug);
        }
    }
    let loader = instance
        .loader
        .as_ref()
        .map(|l| l.kind.name().to_lowercase());
    let mut facets = vec![
        vec!["project_type:mod".to_string()],
        vec![format!("versions:{}", instance.minecraft_version())],
    ];
    if let Some(loader) = loader {
        facets.push(vec![format!("categories:{loader}")]);
    }
    #[derive(serde::Deserialize)]
    struct Hit {
        slug: String,
    }
    #[derive(serde::Deserialize)]
    struct Hits {
        hits: Vec<Hit>,
    }
    if let Ok(response) = http
        .get("https://api.modrinth.com/v2/search")
        .query(&[
            ("query", id.replace(['_', '-'], " ")),
            ("facets", serde_json::to_string(&facets).unwrap_or_default()),
            ("limit", "3".into()),
        ])
        .send()
        .await
        && let Ok(hits) = response.json::<Hits>().await
    {
        for hit in hits.hits {
            if !out.contains(&hit.slug) {
                out.push(hit.slug);
            }
        }
    }
    out
}

fn jar_names(dir: &Path) -> HashSet<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect()
}

/// Install what the instance's mods require but don't have, from Modrinth. Each candidate
/// is checked: it stays only if it really provides the missing id. Returns the names
/// installed and what's still missing (not on Modrinth for this version and loader).
pub async fn install_missing(
    http: &reqwest::Client,
    paths: &Paths,
    instance: &Instance,
    progress: &ProgressFn,
) -> Result<(Vec<String>, Vec<Missing>)> {
    let mods_dir = instance.mods_dir(paths);
    let mut installed = Vec::new();
    let mut tried = HashSet::new();
    // Installing one mod can reveal more needs (its own dependencies' dependencies).
    for _ in 0..4 {
        let wanted: Vec<Missing> = missing(&mods_dir)
            .into_iter()
            .filter(|m| !tried.contains(&m.id))
            .collect();
        if wanted.is_empty() {
            break;
        }
        for need in wanted {
            tried.insert(need.id.clone());
            for project in candidates(http, instance, &need.id).await {
                let before = jar_names(&mods_dir);
                let Ok(titles) =
                    crate::mods::install(http, paths, instance, &project, progress).await
                else {
                    continue;
                };
                if !missing(&mods_dir).iter().any(|m| m.id == need.id) {
                    installed.extend(titles);
                    break;
                }
                // Not the right mod after all: take it back out.
                for file in jar_names(&mods_dir).difference(&before) {
                    let _ = crate::mods::remove(paths, instance, file);
                }
            }
        }
    }
    Ok((installed, missing(&mods_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(dir: &Path, name: &str, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(dir.join(name)).unwrap());
        for (path, bytes) in files {
            zip.start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn nested_jar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in files {
            zip.start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn finds_missing_dependencies_for_every_loader() {
        let dir = std::env::temp_dir().join(format!("nethernode-deps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Fabric mod needing fabric-api (bundled module) and cloth-config (absent).
        jar(&dir, "gun.jar", &[(
            "fabric.mod.json",
            br#"{"id":"gunmod","name":"Gun Mod","depends":{"fabricloader":"*","minecraft":"1.21.1","fabric-api-base":"*","cloth-config":"*"}}"#,
        )]);
        let module = nested_jar(&[("fabric.mod.json", br#"{"id":"fabric-api-base"}"#)]);
        jar(
            &dir,
            "fabric-api.jar",
            &[
                (
                    "fabric.mod.json",
                    br#"{"id":"fabric-api","provides":["fabric"]}"#,
                ),
                ("META-INF/jars/fabric-api-base.jar", &module),
            ],
        );
        // Forge mod needing geckolib (mandatory) and jei (optional).
        jar(&dir, "cod.jar", &[(
            "META-INF/mods.toml",
            b"modLoader=\"javafml\"\n[[mods]]\nmodId=\"cod\"\ndisplayName=\"Call of Duty\"\n[[dependencies.cod]]\nmodId=\"forge\"\nmandatory=true\n[[dependencies.cod]]\nmodId=\"geckolib\"\nmandatory=true\nside=\"BOTH\"\n[[dependencies.cod]]\nmodId=\"jei\"\nmandatory=false\n",
        )]);
        // NeoForge style.
        jar(&dir, "neo.jar", &[(
            "META-INF/neoforge.mods.toml",
            b"[[mods]]\nmodId=\"neomod\"\n[[dependencies.neomod]]\nmodId=\"curios\"\ntype=\"required\"\n[[dependencies.neomod]]\nmodId=\"jade\"\ntype=\"optional\"\n",
        )]);
        let missing = missing(&dir);
        let ids: Vec<&str> = missing.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["cloth-config", "curios", "geckolib"]);
        assert_eq!(missing[2].needed_by, ["Call of Duty"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
