//! Minecraft's own textures, read at runtime from a game jar the launcher already
//! downloaded from Mojang. netherNode never ships Mojang's art; before any version is
//! downloaded, the UI falls back to its own procedural stand-ins.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use nethernode_core::loader::LoaderKind;
use nethernode_core::paths::Paths;

#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct GameTextures {
    pub obsidian: Option<Image>,
    /// Vertical strip of 16x16 animation frames.
    pub portal: Option<Image>,
    /// Instance icons, by name (see [`icon_name`]).
    pub icons: HashMap<&'static str, iced::widget::image::Handle>,
}

/// Block textures used as instance icons, per loader. (name, path after `textures/`)
const ICONS: &[(&str, &str)] = &[
    ("vanilla", "block/grass_block_side"),
    ("fabric", "block/crafting_table_front"),
    ("quilt", "block/loom_front"),
    ("forge", "block/anvil"),
    ("neoforge", "block/smithing_table_front"),
    ("optifine", "block/glass"),
];

pub fn icon_name(loader: Option<LoaderKind>) -> &'static str {
    match loader {
        None => "vanilla",
        Some(LoaderKind::Fabric) => "fabric",
        Some(LoaderKind::Quilt) => "quilt",
        Some(LoaderKind::Forge) => "forge",
        Some(LoaderKind::NeoForge) => "neoforge",
        Some(LoaderKind::OptiFine) => "optifine",
    }
}

fn decode(bytes: &[u8]) -> Option<Image> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    buf.truncate(frame.buffer_size());
    let rgba = match frame.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None, // expanded by normalize_to_color8
    };
    Some(Image {
        width: frame.width,
        height: frame.height,
        rgba,
    })
}

/// A vanilla game jar on disk, newest first. Loader versions are skipped: their jar is a
/// link to the vanilla one anyway.
fn find_game_jar(paths: &Paths) -> Option<PathBuf> {
    let mut jars: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(paths.versions())
        .ok()?
        .flatten()
        .filter_map(|dir| {
            let id = dir.file_name().to_str()?.to_string();
            let jar = dir.path().join(format!("{id}.jar"));
            let modified = jar.metadata().ok()?.modified().ok()?;
            Some((modified, jar))
        })
        .collect();
    jars.sort_by_key(|a| std::cmp::Reverse(a.0));
    jars.into_iter().map(|(_, jar)| jar).next()
}

fn read(zip: &mut zip::ZipArchive<std::fs::File>, path: &str) -> Option<Image> {
    // 1.13+ uses textures/block, older versions textures/blocks.
    let candidates = [
        format!("assets/minecraft/textures/{path}.png"),
        format!(
            "assets/minecraft/textures/{}.png",
            path.replacen("block/", "blocks/", 1)
        ),
    ];
    for name in candidates {
        if let Ok(mut entry) = zip.by_name(&name) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).ok()?;
            return decode(&bytes);
        }
    }
    None
}

/// Load the textures from the newest downloaded game jar, if there is one.
pub fn load(paths: &Paths) -> Option<GameTextures> {
    load_from(&find_game_jar(paths)?)
}

fn load_from(jar: &Path) -> Option<GameTextures> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    let mut textures = GameTextures {
        obsidian: read(&mut zip, "block/obsidian"),
        portal: read(&mut zip, "block/nether_portal").or_else(|| read(&mut zip, "block/portal")),
        icons: HashMap::new(),
    };
    for (name, path) in ICONS {
        if let Some(img) = read(&mut zip, path) {
            // Animated textures are strips; the icon is the first square frame.
            let side = img.width.min(img.height);
            let rgba = img.rgba[..(side * side * 4) as usize].to_vec();
            textures.icons.insert(
                name,
                iced::widget::image::Handle::from_rgba(side, side, rgba),
            );
        }
    }
    (textures.obsidian.is_some() || textures.portal.is_some()).then_some(textures)
}
