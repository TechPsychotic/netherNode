//! Fabric and Quilt: their meta servers hand out ready-made launcher profiles.

use serde::Deserialize;

use super::{LoaderKind, LoaderVersion};
use crate::download;
use crate::error::{Result, other};
use crate::paths::{Paths, ensure_parent};

fn meta(kind: LoaderKind) -> &'static str {
    match kind {
        LoaderKind::Quilt => "https://meta.quiltmc.org/v3",
        _ => "https://meta.fabricmc.net/v2",
    }
}

#[derive(Deserialize)]
struct Entry {
    loader: LoaderInfo,
}

#[derive(Deserialize)]
struct LoaderInfo {
    version: String,
    /// Fabric marks stable builds; Quilt doesn't, so we infer it from the version name.
    stable: Option<bool>,
}

pub async fn versions(
    http: &reqwest::Client,
    kind: LoaderKind,
    mc: &str,
) -> Result<Vec<LoaderVersion>> {
    let url = format!("{}/versions/loader/{mc}", meta(kind));
    let entries: Vec<Entry> = download::get_json(http, &url).await?;
    let mut out: Vec<LoaderVersion> = entries
        .into_iter()
        .map(|e| LoaderVersion {
            stable: e
                .loader
                .stable
                .unwrap_or(!e.loader.version.contains("beta") && !e.loader.version.contains("pre")),
            version: e.loader.version,
        })
        .collect();
    // Quilt's list isn't strictly ordered; sort newest first by the numbers in the version.
    let key = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter_map(|n| n.parse().ok())
            .collect()
    };
    out.sort_by_key(|v| std::cmp::Reverse(key(&v.version)));
    Ok(out)
}

/// Save the loader's profile as `versions/<id>/<id>.json`; it inherits the vanilla version.
pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    kind: LoaderKind,
    mc: &str,
    version: &str,
) -> Result<String> {
    let url = format!("{}/versions/loader/{mc}/{version}/profile/json", meta(kind));
    let profile: serde_json::Value = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = profile["id"]
        .as_str()
        .ok_or_else(|| other(format!("{kind} returned a profile without an id")))?
        .to_string();
    let file = paths.version_json(&id);
    ensure_parent(&file)?;
    std::fs::write(&file, serde_json::to_vec_pretty(&profile)?)?;
    Ok(id)
}
