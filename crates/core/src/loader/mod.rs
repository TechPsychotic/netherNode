//! Mod loaders: listing their versions and installing them as version JSONs that
//! `inheritsFrom` the vanilla version, so the normal launch path handles them.

mod fabric;
pub(crate) mod forge;
pub mod optifine;

use serde::{Deserialize, Serialize};

use crate::download::ProgressFn;
use crate::error::{Result, other};
use crate::paths::Paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
    OptiFine,
}

impl LoaderKind {
    pub const ALL: [LoaderKind; 5] = [
        Self::Fabric,
        Self::Quilt,
        Self::Forge,
        Self::NeoForge,
        Self::OptiFine,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Fabric => "Fabric",
            Self::Quilt => "Quilt",
            Self::Forge => "Forge",
            Self::NeoForge => "NeoForge",
            Self::OptiFine => "OptiFine",
        }
    }

    /// Modrinth loader names whose mods run on this loader.
    pub fn mod_loaders(self) -> &'static [&'static str] {
        match self {
            Self::Fabric => &["fabric"],
            // Quilt runs most Fabric mods.
            Self::Quilt => &["quilt", "fabric"],
            Self::Forge => &["forge"],
            Self::NeoForge => &["neoforge"],
            Self::OptiFine => &[],
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|k| k.name().eq_ignore_ascii_case(s))
    }
}

impl std::fmt::Display for LoaderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The loader an instance runs, recorded so mods can be matched to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loader {
    pub kind: LoaderKind,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderVersion {
    pub version: String,
    /// Stable / recommended builds; others are betas or "latest" builds.
    pub stable: bool,
}

/// Loader versions available for Minecraft `mc`, newest first. Empty means unsupported.
pub async fn versions(
    http: &reqwest::Client,
    kind: LoaderKind,
    mc: &str,
) -> Result<Vec<LoaderVersion>> {
    match kind {
        LoaderKind::Fabric | LoaderKind::Quilt => fabric::versions(http, kind, mc).await,
        LoaderKind::Forge | LoaderKind::NeoForge => forge::versions(http, kind, mc).await,
        LoaderKind::OptiFine => Err(other(
            "OptiFine can't be downloaded automatically; install it from a jar you downloaded",
        )),
    }
}

/// The version to pick when the user doesn't choose: newest stable, else newest.
pub fn recommended(versions: &[LoaderVersion]) -> Option<&LoaderVersion> {
    versions.iter().find(|v| v.stable).or(versions.first())
}

/// Install `kind` `version` for Minecraft `mc`. Returns the version id to launch.
pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    kind: LoaderKind,
    mc: &str,
    version: &str,
    progress: &ProgressFn,
) -> Result<String> {
    match kind {
        LoaderKind::Fabric | LoaderKind::Quilt => {
            fabric::install(http, paths, kind, mc, version).await
        }
        LoaderKind::Forge | LoaderKind::NeoForge => {
            forge::install(http, paths, kind, mc, version, progress).await
        }
        LoaderKind::OptiFine => Err(other("use loader::optifine::install with the OptiFine jar")),
    }
}

/// Put `kind` on `instance` (at `version`, or the recommended one) and save it.
/// Fabric, Quilt, Forge and NeoForge only; OptiFine goes through [`apply_optifine`].
pub async fn apply(
    http: &reqwest::Client,
    paths: &Paths,
    instance: &mut crate::instance::Instance,
    kind: LoaderKind,
    version: Option<&str>,
    progress: &ProgressFn,
) -> Result<()> {
    let mc = instance.minecraft_version().to_string();
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            let all = versions(http, kind, &mc).await?;
            recommended(&all)
                .ok_or_else(|| other(format!("{kind} has no release for Minecraft {mc}")))?
                .version
                .clone()
        }
    };
    let id = install(http, paths, kind, &mc, &version, progress).await?;
    instance.minecraft = Some(mc);
    instance.version = id;
    instance.loader = Some(Loader { kind, version });
    instance.save(paths)
}

/// Install OptiFine from a jar the user downloaded, and put it on `instance`.
pub async fn apply_optifine(
    http: &reqwest::Client,
    paths: &Paths,
    instance: &mut crate::instance::Instance,
    jar: &std::path::Path,
    progress: &ProgressFn,
) -> Result<()> {
    let mc = instance.minecraft_version().to_string();
    let id = optifine::install(http, paths, jar, &mc, progress).await?;
    let edition = id
        .split_once("-OptiFine_")
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();
    instance.minecraft = Some(mc);
    instance.version = id;
    instance.loader = Some(Loader {
        kind: LoaderKind::OptiFine,
        version: edition,
    });
    instance.save(paths)
}

/// Back to plain vanilla. Mods stay in the folder but won't load.
pub fn remove(paths: &Paths, instance: &mut crate::instance::Instance) -> Result<()> {
    if let Some(mc) = instance.minecraft.take() {
        instance.version = mc;
    }
    instance.loader = None;
    instance.save(paths)
}

/// Every `<version>` in a Maven `maven-metadata.xml`, in file order (oldest first).
pub(crate) fn maven_versions(xml: &str) -> Vec<String> {
    xml.split("<version>")
        .skip(1)
        .filter_map(|s| {
            s.split_once("</version>")
                .map(|(v, _)| v.trim().to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_maven_metadata() {
        let xml = "<metadata><versioning><versions><version>1.0</version>\n<version>1.1</version></versions></versioning></metadata>";
        assert_eq!(maven_versions(xml), ["1.0", "1.1"]);
    }

    #[test]
    fn loader_names_round_trip() {
        for kind in LoaderKind::ALL {
            assert_eq!(LoaderKind::parse(&kind.name().to_lowercase()), Some(kind));
        }
    }
}
