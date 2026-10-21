//! Instances: isolated game directories, each pinned to one version.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, other};
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
        })
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

    pub fn list(paths: &Paths) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(paths.instances()) else {
            return Ok(out);
        };
        for entry in entries.flatten() {
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

#[cfg(test)]
mod tests {
    use super::validate_name;

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
}
