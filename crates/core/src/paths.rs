use std::path::{Path, PathBuf};

/// On-disk layout of the launcher data directory.
///
/// ```text
/// <root>/
///   meta/version_manifest_v2.json
///   versions/<id>/<id>.json, <id>.jar
///   libraries/<maven path>
///   assets/{indexes,objects,virtual,log_configs}
///   runtimes/<platform>/<component>/
///   instances/<name>/{instance.json,.minecraft,natives}
///   accounts.json
/// ```
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$NETHERNODE_HOME`, else the platform data dir (e.g. `~/.local/share/nethernode`).
    pub fn default_root() -> PathBuf {
        if let Some(home) = std::env::var_os("NETHERNODE_HOME") {
            return PathBuf::from(home);
        }
        directories::ProjectDirs::from("", "", "nethernode")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".nethernode"))
    }

    pub fn meta(&self) -> PathBuf {
        self.root.join("meta")
    }
    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }
    pub fn version_json(&self, id: &str) -> PathBuf {
        self.versions().join(id).join(format!("{id}.json"))
    }
    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.versions().join(id).join(format!("{id}.jar"))
    }
    pub fn libraries(&self) -> PathBuf {
        self.root.join("libraries")
    }
    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }
    pub fn runtimes(&self) -> PathBuf {
        self.root.join("runtimes")
    }
    pub fn instances(&self) -> PathBuf {
        self.root.join("instances")
    }
    pub fn accounts_file(&self) -> PathBuf {
        self.root.join("accounts.json")
    }
}

pub(crate) fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
