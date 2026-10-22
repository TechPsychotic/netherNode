//! Minecraft's own textures, read at runtime from a game jar the launcher already
//! downloaded from Mojang. netherNode never ships Mojang's art; before any version is
//! downloaded, the UI falls back to its own procedural stand-ins.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

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

/// Block textures used as instance icons, per loader: (name, paths after `textures/`, the
/// first one found wins). Old versions name some blocks differently or don't have them.
const ICONS: &[(&str, &[&str])] = &[
    ("vanilla", &["block/grass_block_side", "blocks/grass_side"]),
    ("fabric", &["block/crafting_table_front"]),
    ("quilt", &["block/loom_front"]),
    ("forge", &["block/furnace_front_on"]),
    ("neoforge", &["block/smithing_table_front"]),
    ("optifine", &["block/glass"]),
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

/// A skin's face (front of the head plus the hat layer), 8×8, for the Accounts screen.
pub fn skin_face(png: &[u8]) -> Option<iced::widget::image::Handle> {
    let img = decode(png)?;
    let px = |x: u32, y: u32| {
        let i = ((y * img.width + x) * 4) as usize;
        [
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        ]
    };
    let mut face = Vec::with_capacity(8 * 8 * 4);
    for y in 8..16 {
        for x in 8..16 {
            let [r, g, b, _] = px(x, y);
            let [hr, hg, hb, ha] = px(x + 32, y);
            let a = u16::from(ha);
            let mix = |base: u8, top: u8| {
                ((u16::from(base) * (255 - a) + u16::from(top) * a) / 255) as u8
            };
            face.extend_from_slice(&[mix(r, hr), mix(g, hg), mix(b, hb), 255]);
        }
    }
    Some(iced::widget::image::Handle::from_rgba(8, 8, face))
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
/// Downloaded game jars, newest first.
fn game_jars(paths: &Paths) -> Vec<PathBuf> {
    let Ok(dirs) = std::fs::read_dir(paths.versions()) else {
        return Vec::new();
    };
    let mut jars: Vec<(std::time::SystemTime, PathBuf)> = dirs
        .flatten()
        .filter_map(|dir| {
            let id = dir.file_name().to_str()?.to_string();
            let jar = dir.path().join(format!("{id}.jar"));
            let modified = jar.metadata().ok()?.modified().ok()?;
            Some((modified, jar))
        })
        .collect();
    jars.sort_by_key(|a| std::cmp::Reverse(a.0));
    jars.into_iter().map(|(_, jar)| jar).collect()
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

/// Load the textures from downloaded game jars, newest first. A texture missing from one
/// jar (an old version may not have a loom, say) is taken from the next.
pub fn load(paths: &Paths) -> Option<GameTextures> {
    let mut textures = GameTextures::default();
    for jar in game_jars(paths) {
        let Some(mut zip) = std::fs::File::open(&jar)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
        else {
            continue;
        };
        if textures.obsidian.is_none() {
            textures.obsidian = read(&mut zip, "block/obsidian");
        }
        if textures.portal.is_none() {
            textures.portal =
                read(&mut zip, "block/nether_portal").or_else(|| read(&mut zip, "block/portal"));
        }
        for (name, candidates) in ICONS {
            if textures.icons.contains_key(name) {
                continue;
            }
            if let Some(img) = candidates.iter().find_map(|path| read(&mut zip, path)) {
                // Animated textures are strips; the icon is the first square frame.
                let side = img.width.min(img.height);
                let rgba = img.rgba[..(side * side * 4) as usize].to_vec();
                textures.icons.insert(
                    name,
                    iced::widget::image::Handle::from_rgba(side, side, rgba),
                );
            }
        }
        if textures.obsidian.is_some()
            && textures.portal.is_some()
            && textures.icons.len() == ICONS.len()
        {
            break;
        }
    }
    (textures.obsidian.is_some() || textures.portal.is_some() || !textures.icons.is_empty())
        .then_some(textures)
}

/// netherNode's own 16×16 pixel-art icon for a loader, used until the game's textures are
/// available (nothing here comes from Minecraft).
pub fn fallback_icon(name: &str) -> iced::widget::image::Handle {
    static ICONS: std::sync::LazyLock<HashMap<&'static str, iced::widget::image::Handle>> =
        std::sync::LazyLock::new(|| {
            [
                "vanilla", "fabric", "quilt", "forge", "neoforge", "optifine",
            ]
            .into_iter()
            .map(|name| {
                (
                    name,
                    iced::widget::image::Handle::from_rgba(16, 16, draw_icon(name)),
                )
            })
            .collect()
        });
    ICONS
        .get(name)
        .or_else(|| ICONS.get("vanilla"))
        .cloned()
        .expect("vanilla icon")
}

/// Cheap deterministic per-pixel noise in 0..=255.
fn noise(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(374_761_393)
        ^ y.wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) & 0xff
}

fn shade([r, g, b]: [u8; 3], amount: i32) -> [u8; 4] {
    let f = |c: u8| (i32::from(c) + amount).clamp(0, 255) as u8;
    [f(r), f(g), f(b), 255]
}

fn draw_icon(name: &str) -> Vec<u8> {
    let mut px = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let n = noise(x, y, name.len() as u32) as i32 / 12 - 10;
            let edge = x == 0 || y == 0 || x == 15 || y == 15;
            let c = match name {
                // Grass over dirt, with a ragged edge.
                "vanilla" => {
                    if y < 3 || (y == 3 && noise(x, 0, 7).is_multiple_of(2)) {
                        shade([92, 160, 60], n)
                    } else if noise(x, y, 3).is_multiple_of(9) {
                        shade([120, 120, 120], n)
                    } else {
                        shade([134, 96, 67], n)
                    }
                }
                // A workbench: planks with a dark grid of tools.
                "fabric" => {
                    if edge {
                        shade([92, 62, 36], n)
                    } else if y < 4 {
                        shade([170, 120, 70], n + if x % 4 == 0 { -30 } else { 0 })
                    } else if (x == 5 || x == 10) && y > 5 {
                        shade([70, 70, 75], n)
                    } else {
                        shade([150, 104, 60], n + if y % 4 == 0 { -25 } else { 0 })
                    }
                }
                // A patchwork quilt.
                "quilt" => {
                    let patch = (x / 4 + y / 4) % 3;
                    let base = [[150, 90, 200], [230, 200, 120], [90, 150, 220]][patch as usize];
                    if x % 4 == 0 || y % 4 == 0 {
                        shade([235, 230, 220], n / 2)
                    } else {
                        shade(base, n)
                    }
                }
                // A stone furnace with fire inside.
                "forge" => {
                    if (4..12).contains(&x) && (8..14).contains(&y) {
                        let hot = 13 - y + noise(x, y, 11) % 3;
                        if hot > 3 {
                            shade([250, 200, 60], n)
                        } else {
                            shade([230, 90, 30], n)
                        }
                    } else if (3..13).contains(&x) && (7..15).contains(&y) {
                        shade([40, 36, 36], n)
                    } else {
                        shade([125, 125, 125], n + if y % 5 == 0 { -25 } else { 0 })
                    }
                }
                // An anvil-topped table in warm orange.
                "neoforge" => {
                    if y < 4 && (2..14).contains(&x) {
                        shade([60, 62, 70], n)
                    } else if edge {
                        shade([110, 60, 30], n)
                    } else {
                        shade([214, 120, 50], n + if (x + y) % 5 == 0 { -30 } else { 0 })
                    }
                }
                // A glass pane with a highlight.
                _ => {
                    if edge {
                        shade([190, 225, 240], n / 2)
                    } else if x + 2 == y || x + 3 == y {
                        shade([245, 252, 255], 0)
                    } else {
                        [150, 200, 230, 70]
                    }
                }
            };
            px.extend_from_slice(&c);
        }
    }
    px
}
