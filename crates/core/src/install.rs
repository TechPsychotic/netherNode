//! Downloads everything a resolved version needs: client jar, libraries, natives, assets, log config.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::download::{self, Download, ProgressFn};
use crate::error::{Result, other};
use crate::paths::{Paths, ensure_parent};
use crate::rules::{self, Environment};
use crate::version::VersionJson;

const RESOURCES_URL: &str = "https://resources.download.minecraft.net";

/// Everything the launch step needs to know about the files on disk.
#[derive(Debug, Clone)]
pub struct Installed {
    pub classpath: Vec<PathBuf>,
    pub natives_dir: PathBuf,
    pub assets_root: PathBuf,
    pub assets_index_name: String,
    /// Directory for `${game_assets}`: the virtual folder for legacy indexes, else the assets root.
    pub game_assets: PathBuf,
    /// The log4j argument with `${path}` already filled in.
    pub logging_arg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
    #[serde(rename = "virtual", default)]
    is_virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    version: &VersionJson,
    env: &Environment,
    game_dir: &Path,
    natives_dir: &Path,
    progress: &ProgressFn,
) -> Result<Installed> {
    let mut downloads = Vec::new();
    let mut classpath = Vec::new();
    let mut native_jars = Vec::new();

    for lib in version
        .libraries
        .iter()
        .filter(|l| rules::allowed(&l.rules, env))
    {
        if let Some(native) = lib.native_artifact(env.os_name, env.arch_bits()) {
            let dest = paths
                .libraries()
                .join(native.path.as_deref().unwrap_or_default());
            let exclude = lib.extract.clone().unwrap_or_default().exclude;
            native_jars.push((dest.clone(), exclude));
            downloads.push(Download {
                url: native.url,
                dest,
                sha1: native.sha1,
                size: native.size,
            });
        }
        if let Some(artifact) = lib.artifact() {
            if artifact.url.is_empty() {
                continue;
            }
            let dest = paths
                .libraries()
                .join(artifact.path.as_deref().unwrap_or_default());
            if !classpath.contains(&dest) {
                classpath.push(dest.clone());
            }
            downloads.push(Download {
                url: artifact.url,
                dest,
                sha1: artifact.sha1,
                size: artifact.size,
            });
        }
    }

    let jar = paths.version_jar(&version.jar);
    if let Some(client) = version.downloads.as_ref().and_then(|d| d.client.clone()) {
        downloads.push(Download {
            url: client.url,
            dest: jar.clone(),
            sha1: client.sha1,
            size: client.size,
        });
    } else if !jar.exists() {
        return Err(other(format!("no client jar for '{}'", version.jar)));
    }
    classpath.push(jar);

    let logging_arg = match version.logging.as_ref().and_then(|l| l.client.clone()) {
        Some(cfg) => {
            let dest = paths.assets().join("log_configs").join(&cfg.file.id);
            downloads.push(Download {
                url: cfg.file.url,
                dest: dest.clone(),
                sha1: cfg.file.sha1,
                size: cfg.file.size,
            });
            Some(cfg.argument.replace("${path}", &dest.to_string_lossy()))
        }
        None => None,
    };

    download::fetch_all(http, "Libraries", downloads, progress).await?;

    // Per instance (and guarded by the instance lock), so re-extracting never touches
    // libraries another running game has loaded; Windows refuses to delete those.
    extract_natives(&native_jars, natives_dir)?;

    let (assets_index_name, game_assets) =
        install_assets(http, paths, version, game_dir, progress).await?;

    Ok(Installed {
        classpath,
        natives_dir: natives_dir.to_path_buf(),
        assets_root: paths.assets(),
        assets_index_name,
        game_assets,
        logging_arg,
    })
}

/// Fresh extraction every launch so stale natives from an older install never linger.
fn extract_natives(jars: &[(PathBuf, Vec<String>)], dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    for (jar, exclude) in jars {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let Some(name) = entry.enclosed_name() else {
                continue;
            };
            let name_str = name.to_string_lossy().replace('\\', "/");
            if entry.is_dir() || exclude.iter().any(|e| name_str.starts_with(e.as_str())) {
                continue;
            }
            let out = dir.join(&name);
            ensure_parent(&out)?;
            std::io::copy(&mut entry, &mut std::fs::File::create(out)?)?;
        }
    }
    Ok(())
}

async fn install_assets(
    http: &reqwest::Client,
    paths: &Paths,
    version: &VersionJson,
    game_dir: &Path,
    progress: &ProgressFn,
) -> Result<(String, PathBuf)> {
    let Some(index_ref) = &version.asset_index else {
        let id = version.assets.clone().unwrap_or_else(|| "legacy".into());
        return Ok((id, paths.assets()));
    };

    let index_file = paths
        .assets()
        .join("indexes")
        .join(format!("{}.json", index_ref.id));
    let index_download = Download {
        url: index_ref.url.clone(),
        dest: index_file.clone(),
        sha1: Some(index_ref.sha1.clone()),
        size: index_ref.size,
    };
    let stale = download::sha1_file(&index_file).map_or(true, |h| h != index_ref.sha1);
    if stale {
        download::fetch(http, &index_download).await?;
    }
    let index: AssetIndex = serde_json::from_slice(&std::fs::read(&index_file)?)?;

    let objects_dir = paths.assets().join("objects");
    let object_path = |hash: &str| objects_dir.join(&hash[..2]).join(hash);
    let downloads = index
        .objects
        .values()
        .map(|o| Download {
            url: format!("{RESOURCES_URL}/{}/{}", &o.hash[..2], o.hash),
            dest: object_path(&o.hash),
            sha1: Some(o.hash.clone()),
            size: Some(o.size),
        })
        .collect();
    download::fetch_all(http, "Assets", downloads, progress).await?;

    // Old versions read assets by name instead of by hash, so lay them out as plain files.
    let mut game_assets = paths.assets();
    let legacy_targets = [
        index
            .is_virtual
            .then(|| paths.assets().join("virtual").join(&index_ref.id)),
        index.map_to_resources.then(|| game_dir.join("resources")),
    ];
    for target in legacy_targets.into_iter().flatten() {
        for (name, obj) in &index.objects {
            let dest = target.join(name);
            if !dest.exists() {
                ensure_parent(&dest)?;
                std::fs::copy(object_path(&obj.hash), &dest)?;
            }
        }
        if index.is_virtual {
            game_assets = paths.assets().join("virtual").join(&index_ref.id);
        }
    }

    Ok((index_ref.id.clone(), game_assets))
}
