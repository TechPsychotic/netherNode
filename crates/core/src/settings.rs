//! Launcher-wide settings, kept in the OS config folder (not the data folder, so moving the
//! data never loses them). With `$NETHERNODE_HOME` set they live there instead, which keeps
//! tests and portable setups self-contained.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, other};
use crate::paths::{Paths, ensure_parent};

/// How the GUI draws itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    /// GPU, except on machines with 4 GB of RAM or less.
    #[default]
    Auto,
    Gpu,
    /// Software rendering: lighter, but no animated background.
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Game files (versions, libraries, assets, Java). `None` = the platform default.
    pub data_dir: Option<PathBuf>,
    /// Instances and their worlds. `None` = `<data_dir>/instances`.
    pub instances_dir: Option<PathBuf>,
    pub renderer: Renderer,
    pub animated_background: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: None,
            instances_dir: None,
            renderer: Renderer::Auto,
            animated_background: true,
        }
    }
}

impl Settings {
    pub fn file() -> PathBuf {
        if let Some(home) = std::env::var_os("NETHERNODE_HOME") {
            return PathBuf::from(home).join("settings.json");
        }
        directories::ProjectDirs::from("", "", "nethernode")
            .map(|d| d.config_dir().join("settings.json"))
            .unwrap_or_else(|| PathBuf::from("nethernode-settings.json"))
    }

    pub fn load() -> Self {
        std::fs::read(Self::file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let file = Self::file();
        ensure_parent(&file)?;
        std::fs::write(file, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn paths(&self) -> Paths {
        // A NETHERNODE_HOME override always wins (tests, portable installs).
        if let Some(home) = std::env::var_os("NETHERNODE_HOME") {
            let paths = Paths::new(home);
            return match &self.instances_dir {
                Some(dir) => paths.with_instances_dir(dir),
                None => paths,
            };
        }
        Paths::from_settings(self)
    }

    /// Whether the GUI should use the GPU renderer on this machine.
    pub fn use_gpu(&self, total_ram_mb: u64) -> bool {
        match self.renderer {
            Renderer::Gpu => true,
            Renderer::Cpu => false,
            Renderer::Auto => total_ram_mb > 4 * 1024,
        }
    }
}

/// Move a folder's contents to `to` (rename when possible, otherwise copy then delete).
/// `to` must be empty or not exist, and must not be inside `from`.
pub fn move_dir(from: &Path, to: &Path) -> Result<()> {
    if !from.exists() {
        std::fs::create_dir_all(to)?;
        return Ok(());
    }
    let from_c = from.canonicalize()?;
    if let Ok(to_c) = to.canonicalize()
        && to_c == from_c
    {
        return Ok(());
    }
    let to_abs = if to.is_absolute() {
        to.to_path_buf()
    } else {
        std::env::current_dir()?.join(to)
    };
    if to_abs.starts_with(&from_c) {
        return Err(other("the new folder can't be inside the current one"));
    }
    if to.exists() && std::fs::read_dir(to)?.next().is_some() {
        return Err(other(format!(
            "{} isn't empty; choose an empty folder",
            to.display()
        )));
    }
    ensure_parent(to)?;
    let _ = std::fs::remove_dir(to); // empty: let rename create it
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    // Different drive: copy everything, then remove the old copy.
    copy_dir(from, to)?;
    std::fs::remove_dir_all(from)?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &target)?;
            #[cfg(not(unix))]
            std::fs::copy(entry.path(), &target).map(|_| ())?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_renderer_threshold() {
        let s = Settings::default();
        assert!(s.use_gpu(8 * 1024));
        assert!(!s.use_gpu(4 * 1024));
        assert!(
            Settings {
                renderer: Renderer::Gpu,
                ..Settings::default()
            }
            .use_gpu(2048)
        );
        assert!(
            !Settings {
                renderer: Renderer::Cpu,
                ..Settings::default()
            }
            .use_gpu(32768)
        );
    }

    #[test]
    fn moves_folders_and_refuses_bad_targets() {
        let base =
            std::env::temp_dir().join(format!("nethernode-move-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let from = base.join("a");
        std::fs::create_dir_all(from.join("x/y")).unwrap();
        std::fs::write(from.join("x/y/file.txt"), b"hi").unwrap();

        assert!(move_dir(&from, &from.join("inside")).is_err());
        let busy = base.join("busy");
        std::fs::create_dir_all(&busy).unwrap();
        std::fs::write(busy.join("other"), b"x").unwrap();
        assert!(move_dir(&from, &busy).is_err());

        let to = base.join("b");
        move_dir(&from, &to).unwrap();
        assert_eq!(std::fs::read(to.join("x/y/file.txt")).unwrap(), b"hi");
        assert!(!from.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
