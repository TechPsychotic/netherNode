//! Instances: isolated game directories, each pinned to one version.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, other};
use crate::loader::Loader;
use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    /// Version id: vanilla (`1.21.1`) or a mod-loader profile placed in `versions/`.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_path: Option<PathBuf>,
    /// Heap limits in MB. `None` means sized automatically from this machine's RAM at launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_memory_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jvm_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<(u32, u32)>,
    /// The mod loader, if any. `version` is then the loader's version id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<Loader>,
    /// The Minecraft version, when it differs from `version` (i.e. with a loader).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft: Option<String>,
    /// Set when the instance was created from a modpack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modpack: Option<crate::modpack::ModpackInfo>,
}

/// Names must be valid folder names on Windows, macOS and Linux alike.
fn validate_name(name: &str) -> Result<()> {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let bad = name.is_empty()
        || name.len() > 64
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || RESERVED.contains(&stem.as_str())
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if bad {
        Err(other(format!("invalid instance name '{name}'")))
    } else {
        Ok(())
    }
}

impl Instance {
    pub fn new(name: &str, version: &str) -> Result<Self> {
        validate_name(name)?;
        Ok(Self {
            name: name.to_string(),
            version: version.to_string(),
            java_path: None,
            min_memory_mb: None,
            max_memory_mb: None,
            jvm_args: Vec::new(),
            resolution: None,
            loader: None,
            minecraft: None,
            modpack: None,
        })
    }

    pub fn minecraft_version(&self) -> &str {
        self.minecraft.as_deref().unwrap_or(&self.version)
    }

    pub fn mods_dir(&self, paths: &Paths) -> PathBuf {
        self.game_dir(paths).join("mods")
    }

    pub fn dir(&self, paths: &Paths) -> PathBuf {
        paths.instances().join(&self.name)
    }

    /// The game's working directory (saves, mods, options.txt...).
    pub fn game_dir(&self, paths: &Paths) -> PathBuf {
        self.dir(paths).join(".minecraft")
    }

    pub fn load(paths: &Paths, name: &str) -> Result<Self> {
        validate_name(name)?;
        let file = paths.instances().join(name).join("instance.json");
        let bytes =
            std::fs::read(&file).map_err(|_| other(format!("no instance named '{name}'")))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        std::fs::create_dir_all(self.game_dir(paths))?;
        std::fs::write(
            self.dir(paths).join("instance.json"),
            serde_json::to_vec_pretty(self)?,
        )?;
        Ok(())
    }

    pub fn create(paths: &Paths, name: &str, version: &str, low_end: bool) -> Result<Self> {
        let instance = Self::new(name, version)?;
        if instance.dir(paths).exists() {
            return Err(other(format!("instance '{name}' already exists")));
        }
        instance.save(paths)?;
        if low_end {
            std::fs::write(
                instance.game_dir(paths).join("options.txt"),
                crate::memory::LOW_END_OPTIONS,
            )?;
        }
        Ok(instance)
    }

    /// Exclusive per-instance lock, held for as long as the returned file is open. Stops the
    /// same instance from being started twice; the OS releases it if the launcher dies.
    pub fn lock(&self, paths: &Paths) -> Result<std::fs::File> {
        // No truncate: on Windows, truncating a file another process has locked fails.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir(paths).join(".lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => Err(other(format!(
                "instance '{}' is already running",
                self.name
            ))),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }

    pub fn delete(paths: &Paths, name: &str) -> Result<()> {
        let instance = Self::load(paths, name)?;
        std::fs::remove_dir_all(instance.dir(paths))?;
        Ok(())
    }

    /// Marker present while an instance is still being built (e.g. a modpack import).
    /// The builder holds an OS lock on it, which is released even if the process dies.
    pub const INSTALLING: &'static str = ".installing";

    /// Start building this instance: hidden from [`Instance::list`] until the returned
    /// guard is finished with [`Installing::done`].
    pub fn begin_install(&self, paths: &Paths) -> Result<Installing> {
        let path = self.dir(paths).join(Self::INSTALLING);
        let file = std::fs::File::create(&path)?;
        file.try_lock()
            .map_err(|_| other("this instance is already being installed"))?;
        Ok(Installing { _file: file, path })
    }

    /// Remove instances whose install was interrupted (marker present, nobody holding it).
    pub fn clean_interrupted(paths: &Paths) -> Vec<String> {
        let mut removed = Vec::new();
        let Ok(entries) = std::fs::read_dir(paths.instances()) else {
            return removed;
        };
        for entry in entries.flatten() {
            let marker = entry.path().join(Self::INSTALLING);
            let Ok(file) = std::fs::OpenOptions::new().write(true).open(&marker) else {
                continue;
            };
            if file.try_lock().is_ok() {
                drop(file);
                if std::fs::remove_dir_all(entry.path()).is_ok() {
                    removed.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        removed
    }

    pub fn list(paths: &Paths) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(paths.instances()) else {
            return Ok(out);
        };
        for entry in entries.flatten() {
            // Still being built (see begin_install): not ready to show or play.
            if entry.path().join(Self::INSTALLING).exists() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str()
                && let Ok(instance) = Self::load(paths, name)
            {
                out.push(instance);
            }
        }
        out.sort_by_key(|a| a.name.to_lowercase());
        Ok(out)
    }
}

/// Held while an instance is being built; see [`Instance::begin_install`].
pub struct Installing {
    _file: std::fs::File,
    path: PathBuf,
}

impl Installing {
    /// The instance is complete: show it in the list.
    pub fn done(self) -> Result<()> {
        std::fs::remove_file(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Instance, Paths, validate_name};

    #[test]
    fn names_valid_on_every_os() {
        for ok in ["survival", "My Pack 1.20", "fabric-1.21"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            ".hidden",
            "a/b",
            "a\\b",
            "con",
            "NUL.txt",
            "trailing.",
            "trailing ",
            "a:b",
        ] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn interrupted_installs_are_hidden_then_cleaned() {
        let root =
            std::env::temp_dir().join(format!("nethernode-installing-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::new(&root);
        let instance = Instance::create(&paths, "pack", "1.21.1", false).unwrap();

        let guard = instance.begin_install(&paths).unwrap();
        assert!(Instance::list(&paths).unwrap().is_empty());
        // Held by a live install: not touched.
        assert!(Instance::clean_interrupted(&paths).is_empty());
        drop(guard); // as if the process died mid-install
        assert_eq!(Instance::clean_interrupted(&paths), ["pack"]);
        assert!(!instance.dir(&paths).exists());

        let finished = Instance::create(&paths, "done", "1.21.1", false).unwrap();
        finished.begin_install(&paths).unwrap().done().unwrap();
        assert_eq!(Instance::list(&paths).unwrap().len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
