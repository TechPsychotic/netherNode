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
    /// Game files: versions, libraries, assets, Java runtimes, caches.
    pub root: PathBuf,
    /// Instances, including their worlds. Normally `<root>/instances`; can be moved.
    instances_dir: PathBuf,
    /// Servers hosted on this PC, with their worlds. Normally `<root>/servers`.
    servers_dir: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            instances_dir: root.join("instances"),
            servers_dir: root.join("servers"),
            root,
        }
    }

    /// Keep instances (and their worlds) somewhere other than the game files.
    pub fn with_instances_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.instances_dir = dir.into();
        self
    }

    /// The folders chosen in [`crate::settings::Settings`] (or the defaults).
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let mut paths = Self::new(settings.data_dir.clone().unwrap_or_else(Self::default_root));
        if let Some(dir) = &settings.instances_dir {
            paths = paths.with_instances_dir(dir);
        }
        if let Some(dir) = &settings.servers_dir {
            paths = paths.with_servers_dir(dir);
        }
        paths
    }

    pub fn with_servers_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.servers_dir = dir.into();
        self
    }

    pub fn servers(&self) -> PathBuf {
        self.servers_dir.clone()
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
        self.instances_dir.clone()
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
