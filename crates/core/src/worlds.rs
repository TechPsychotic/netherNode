//! Singleplayer worlds in an instance's `saves/` folder: listing and importing.

use std::path::{Path, PathBuf};

use crate::error::{Result, other};
use crate::instance::Instance;
use crate::paths::{Paths, ensure_parent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct World {
    /// Folder name inside `saves/`.
    pub name: String,
    pub path: PathBuf,
}

pub fn saves_dir(paths: &Paths, instance: &Instance) -> PathBuf {
    instance.game_dir(paths).join("saves")
}

/// Worlds in the instance, newest first.
pub fn list(paths: &Paths, instance: &Instance) -> Vec<World> {
    let Ok(entries) = std::fs::read_dir(saves_dir(paths, instance)) else {
        return Vec::new();
    };
    let mut worlds: Vec<(std::time::SystemTime, World)> = entries
        .flatten()
        .filter(|e| e.path().join("level.dat").is_file())
        .map(|e| {
            let modified = e
                .path()
                .join("level.dat")
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (
                modified,
                World {
                    name: e.file_name().to_string_lossy().into_owned(),
                    path: e.path(),
                },
            )
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.0));
    worlds.into_iter().map(|(_, w)| w).collect()
}

/// A folder name in `saves/` based on `base` that isn't taken yet.
fn free_name(saves: &Path, base: &str) -> String {
    let clean: String = base
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let clean = if clean.is_empty() {
        "World".to_string()
    } else {
        clean
    };
    (1..)
        .map(|n| {
            if n == 1 {
                clean.clone()
            } else {
                format!("{clean} ({n})")
            }
        })
        .find(|n| !saves.join(n).exists())
        .expect("some name is free")
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Import a world from a folder (the one containing `level.dat`) or a `.zip` of one.
/// Returns the folder name it got in `saves/`.
pub fn import(paths: &Paths, instance: &Instance, source: &Path) -> Result<String> {
    let saves = saves_dir(paths, instance);
    std::fs::create_dir_all(&saves)?;
    let is_zip = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));

    if !is_zip {
        if !source.join("level.dat").is_file() {
            return Err(other(format!(
                "{} isn't a Minecraft world (no level.dat inside)",
                source.display()
            )));
        }
        let base = source
            .file_name()
            .map_or("World".into(), |n| n.to_string_lossy().into_owned());
        let name = free_name(&saves, &base);
        copy_dir(source, &saves.join(&name))?;
        return Ok(name);
    }

    let mut zip = zip::ZipArchive::new(std::fs::File::open(source)?)?;
    // The world is wherever level.dat is: the zip root, or one folder down.
    let level = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().and_then(|f| f.enclosed_name()))
        .filter(|p| p.file_name().is_some_and(|n| n == "level.dat"))
        .min_by_key(|p| p.components().count())
        .ok_or_else(|| other("this .zip doesn't contain a Minecraft world (no level.dat)"))?;
    let prefix = level.parent().map(Path::to_path_buf).unwrap_or_default();
    if prefix.components().count() > 1 {
        return Err(other(
            "couldn't find the world inside this .zip (level.dat is nested too deep)",
        ));
    }
    let base = match prefix.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => source
            .file_stem()
            .map_or("World".into(), |n| n.to_string_lossy().into_owned()),
    };
    let name = free_name(&saves, &base);
    let dest = saves.join(&name);
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // enclosed_name rejects absolute paths and `..`, so nothing lands outside `dest`.
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let Ok(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        if rel.as_os_str().is_empty() || entry.is_dir() {
            continue;
        }
        let out = dest.join(rel);
        ensure_parent(&out)?;
        std::io::copy(&mut entry, &mut std::fs::File::create(out)?)?;
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn setup(name: &str) -> (Paths, Instance, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("nethernode-worlds-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::new(root.join("data"));
        let instance = Instance::create(&paths, "w", "1.21.1", false).unwrap();
        (paths, instance, root)
    }

    #[test]
    fn imports_folder_and_avoids_name_clash() {
        let (paths, instance, root) = setup("folder");
        let world = root.join("My World");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"x").unwrap();
        std::fs::write(world.join("region/r.0.0.mca"), b"y").unwrap();

        assert_eq!(import(&paths, &instance, &world).unwrap(), "My World");
        assert_eq!(import(&paths, &instance, &world).unwrap(), "My World (2)");
        assert_eq!(list(&paths, &instance).len(), 2);
        assert!(import(&paths, &instance, &root).is_err()); // no level.dat
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn imports_zip_with_world_in_a_subfolder() {
        let (paths, instance, root) = setup("zip");
        let zip_path = root.join("backup.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("Castle/level.dat", opts).unwrap();
        zip.write_all(b"x").unwrap();
        zip.start_file("Castle/region/r.0.0.mca", opts).unwrap();
        zip.write_all(b"y").unwrap();
        zip.finish().unwrap();

        assert_eq!(import(&paths, &instance, &zip_path).unwrap(), "Castle");
        let saved = saves_dir(&paths, &instance).join("Castle");
        assert!(saved.join("level.dat").is_file() && saved.join("region/r.0.0.mca").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
