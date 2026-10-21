//! Minecraft servers hosted on this PC: Vanilla, Fabric (mods) or Paper (plugins).
//!
//! Servers run in offline mode so offline accounts can join. Friends reach them through
//! netherNode's share tunnel (approval-gated), and an optional whitelist uses the same
//! offline UUIDs the game gives each name.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::download::{self, Download, ProgressFn};
use crate::error::{Result, other};
use crate::java;
use crate::loader::{self, LoaderKind};
use crate::memory::{self, SystemMemory};
use crate::paths::{Paths, ensure_parent};
use crate::rules::Environment;
use crate::version::{self, JavaVersion};

pub const EULA_URL: &str = "https://aka.ms/MinecraftEULA";
const FIRST_PORT: u16 = 25565;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    Vanilla,
    /// Fabric mods (server-side).
    Fabric,
    /// Paper: fast, with Bukkit/Paper plugins.
    Paper,
}

impl ServerKind {
    pub const ALL: [ServerKind; 3] = [Self::Vanilla, Self::Fabric, Self::Paper];

    pub fn name(self) -> &'static str {
        match self {
            Self::Vanilla => "Vanilla",
            Self::Fabric => "Fabric",
            Self::Paper => "Paper",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub minecraft: String,
    pub kind: ServerKind,
    /// Fabric loader version, when `kind` is Fabric.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    /// Heap limit in MB; `None` = automatic, like instances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u32>,
    pub port: u16,
}

fn validate_name(name: &str) -> Result<()> {
    let bad = name.trim().is_empty()
        || name.len() > 64
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if bad {
        Err(other(format!("invalid server name '{name}'")))
    } else {
        Ok(())
    }
}

/// A port no other server of ours uses and that's free right now.
fn free_port(paths: &Paths) -> u16 {
    let taken: Vec<u16> = Server::list(paths).iter().map(|s| s.port).collect();
    (FIRST_PORT..FIRST_PORT + 200)
        .find(|p| !taken.contains(p) && TcpListener::bind((Ipv4Addr::UNSPECIFIED, *p)).is_ok())
        .unwrap_or(FIRST_PORT)
}

impl Server {
    pub fn dir(&self, paths: &Paths) -> PathBuf {
        paths.servers().join(&self.name)
    }

    pub fn jar(&self, paths: &Paths) -> PathBuf {
        self.dir(paths).join("server.jar")
    }

    pub fn world_dir(&self, paths: &Paths) -> PathBuf {
        self.dir(paths).join("world")
    }

    pub fn load(paths: &Paths, name: &str) -> Result<Self> {
        validate_name(name)?;
        let bytes = std::fs::read(paths.servers().join(name).join("netherNode-server.json"))
            .map_err(|_| other(format!("no server named '{name}'")))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = self.dir(paths).join("netherNode-server.json");
        ensure_parent(&file)?;
        std::fs::write(file, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn list(paths: &Paths) -> Vec<Self> {
        let mut out: Vec<Self> = std::fs::read_dir(paths.servers())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| Self::load(paths, e.file_name().to_str()?).ok())
            .collect();
        out.sort_by_key(|s| s.name.to_lowercase());
        out
    }

    /// Make the server folder with sensible offline-mode settings. Downloads happen in
    /// [`install`]; the EULA must be accepted with [`accept_eula`] before it can start.
    pub fn create(paths: &Paths, name: &str, minecraft: &str, kind: ServerKind) -> Result<Self> {
        validate_name(name)?;
        if paths.servers().join(name).exists() {
            return Err(other(format!("a server named '{name}' already exists")));
        }
        let server = Self {
            name: name.to_string(),
            minecraft: minecraft.to_string(),
            kind,
            loader_version: None,
            max_memory_mb: None,
            port: free_port(paths),
        };
        server.save(paths)?;
        let port = server.port.to_string();
        set_properties(
            &server.dir(paths),
            &[
                // Required for offline accounts. Friends still need netherNode's approval.
                ("online-mode", "false"),
                ("server-port", &port),
                ("motd", &format!("{name} (netherNode)")),
                ("max-players", "10"),
                ("difficulty", "normal"),
                ("gamemode", "survival"),
                ("pvp", "true"),
                ("white-list", "false"),
                ("enforce-whitelist", "false"),
            ],
        )?;
        Ok(server)
    }

    pub fn delete(paths: &Paths, name: &str) -> Result<()> {
        let server = Self::load(paths, name)?;
        std::fs::remove_dir_all(server.dir(paths))?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct PaperBuild {
    channel: String,
    downloads: BTreeMap<String, PaperDownload>,
}

#[derive(Deserialize)]
struct PaperDownload {
    url: String,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct FabricInstaller {
    version: String,
    stable: bool,
}

/// Whether `kind` has a server for Minecraft `mc` (for greying out choices).
pub async fn available(http: &reqwest::Client, kind: ServerKind, mc: &str) -> Result<bool> {
    match kind {
        ServerKind::Vanilla => Ok(true),
        ServerKind::Fabric => Ok(!loader::versions(http, LoaderKind::Fabric, mc)
            .await?
            .is_empty()),
        ServerKind::Paper => {
            let resp = http
                .get(format!(
                    "https://fill.papermc.io/v3/projects/paper/versions/{mc}/builds"
                ))
                .send()
                .await?;
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(false);
            }
            let builds: Vec<PaperBuild> = resp.error_for_status()?.json().await?;
            Ok(!builds.is_empty())
        }
    }
}

/// Download (or update) the server jar.
pub async fn install(
    http: &reqwest::Client,
    paths: &Paths,
    server: &mut Server,
    progress: &ProgressFn,
) -> Result<()> {
    let jar = server.jar(paths);
    let mc = server.minecraft.clone();
    let item = match server.kind {
        ServerKind::Vanilla => {
            let version = version::resolve(http, paths, &mc).await?;
            let artifact = version
                .downloads
                .and_then(|d| d.server)
                .ok_or_else(|| other(format!("Minecraft {mc} has no official server")))?;
            Download {
                url: artifact.url,
                dest: jar,
                sha1: artifact.sha1,
                size: artifact.size,
            }
        }
        ServerKind::Fabric => {
            let loader_version = match &server.loader_version {
                Some(v) => v.clone(),
                None => {
                    let all = loader::versions(http, LoaderKind::Fabric, &mc).await?;
                    loader::recommended(&all)
                        .ok_or_else(|| {
                            other(format!("Fabric isn't released for Minecraft {mc} yet"))
                        })?
                        .version
                        .clone()
                }
            };
            let installers: Vec<FabricInstaller> =
                download::get_json(http, "https://meta.fabricmc.net/v2/versions/installer").await?;
            let installer = installers
                .iter()
                .find(|i| i.stable)
                .or(installers.first())
                .ok_or_else(|| other("couldn't find Fabric's server launcher"))?;
            server.loader_version = Some(loader_version.clone());
            server.save(paths)?;
            let url = format!(
                "https://meta.fabricmc.net/v2/versions/loader/{mc}/{loader_version}/{}/server/jar",
                installer.version
            );
            // The launcher jar downloads the vanilla server and Fabric's libraries on first start.
            Download {
                url,
                dest: jar,
                sha1: None,
                size: None,
            }
        }
        ServerKind::Paper => {
            let builds: Vec<PaperBuild> = download::get_json(
                http,
                &format!("https://fill.papermc.io/v3/projects/paper/versions/{mc}/builds"),
            )
            .await?;
            let build = builds
                .iter()
                .find(|b| b.channel == "STABLE")
                .or(builds.first())
                .ok_or_else(|| other(format!("Paper isn't released for Minecraft {mc} yet")))?;
            let file = build
                .downloads
                .get("server:default")
                .ok_or_else(|| other("Paper's download is missing"))?;
            Download {
                url: file.url.clone(),
                dest: jar,
                sha1: None,
                size: file.size,
            }
        }
    };
    let _ = std::fs::remove_file(&item.dest); // always fetch fresh (also updates Paper builds)
    download::fetch_all(http, "Server", vec![item], progress).await
}

pub fn eula_accepted(paths: &Paths, server: &Server) -> bool {
    std::fs::read_to_string(server.dir(paths).join("eula.txt"))
        .is_ok_and(|s| s.lines().any(|l| l.trim() == "eula=true"))
}

/// Record that the player accepted Mojang's EULA (they tick a box saying so).
pub fn accept_eula(paths: &Paths, server: &Server) -> Result<()> {
    let text = format!("# Accepted in netherNode by the player. See {EULA_URL}\neula=true\n");
    std::fs::write(server.dir(paths).join("eula.txt"), text)?;
    Ok(())
}

/// `server.properties` as key -> value.
pub fn properties(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(dir.join("server.properties"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Update keys in `server.properties`, keeping every other line as it was.
pub fn set_properties(dir: &Path, values: &[(&str, &str)]) -> Result<()> {
    let file = dir.join("server.properties");
    ensure_parent(&file)?;
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect();
    for (key, value) in values {
        let line = format!("{key}={value}");
        match lines
            .iter_mut()
            .find(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == *key))
        {
            Some(existing) => *existing = line,
            None => lines.push(line),
        }
    }
    std::fs::write(file, lines.join("\n") + "\n")?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct WhitelistEntry {
    uuid: String,
    name: String,
}

pub fn whitelist(paths: &Paths, server: &Server) -> Vec<String> {
    std::fs::read(server.dir(paths).join("whitelist.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<WhitelistEntry>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.name)
        .collect()
}

/// Turn the whitelist on or off and set its names (offline UUIDs, matching the game).
pub fn set_whitelist(
    paths: &Paths,
    server: &Server,
    enabled: bool,
    names: &[String],
) -> Result<()> {
    for name in names {
        crate::account::validate_name(name)?;
    }
    let entries: Vec<WhitelistEntry> = names
        .iter()
        .map(|n| WhitelistEntry {
            uuid: crate::account::offline_uuid(n),
            name: n.clone(),
        })
        .collect();
    std::fs::write(
        server.dir(paths).join("whitelist.json"),
        serde_json::to_vec_pretty(&entries)?,
    )?;
    let on = if enabled { "true" } else { "false" };
    set_properties(
        &server.dir(paths),
        &[("white-list", on), ("enforce-whitelist", on)],
    )
}

/// Put a world (folder with level.dat, or a .zip of one) in place of the server's world.
/// The old world is kept as `world-before-import-<n>`.
pub fn import_world(paths: &Paths, server: &Server, source: &Path) -> Result<()> {
    let world = server.world_dir(paths);
    if world.exists() {
        let backup = (1..)
            .map(|n| server.dir(paths).join(format!("world-before-import-{n}")))
            .find(|p| !p.exists())
            .expect("some name is free");
        std::fs::rename(&world, backup)?;
    }
    // Reuse the instance importer by importing into a scratch "saves" folder.
    let scratch = server.dir(paths).join(".import");
    let _ = std::fs::remove_dir_all(&scratch);
    let name = crate::worlds::import_into(&scratch, source)?;
    std::fs::rename(scratch.join(name), &world)?;
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

/// Remove terminal colour codes (Paper colours its log) so the console shows plain text.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            // Skip parameters up to and including the final letter.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// How to start a server.
pub struct ServerCommand {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub heap_mb: u64,
}

/// Prepare Java and the command line (downloads Java if needed).
pub async fn command(
    http: &reqwest::Client,
    paths: &Paths,
    server: &Server,
    progress: &ProgressFn,
) -> Result<ServerCommand> {
    if !server.jar(paths).exists() {
        return Err(other("the server isn't installed yet"));
    }
    if !eula_accepted(paths, server) {
        return Err(other(
            "accept the Minecraft EULA in the server's settings first",
        ));
    }
    let vanilla = version::resolve(http, paths, &server.minecraft).await?;
    let required = vanilla.java_version.clone().unwrap_or(JavaVersion {
        component: Some("jre-legacy".into()),
        major_version: 8,
    });
    let java = java::resolve(http, paths, &Environment::current(), &required, progress)
        .await?
        .java;
    let heap = memory::plan_heap(SystemMemory::current(), server.max_memory_mb, None);
    let mut args = vec![
        format!("-Xms{}M", heap.min_mb),
        format!("-Xmx{}M", heap.max_mb),
    ];
    args.extend(memory::gc_flags(&heap, &[]));
    args.extend(["-jar".into(), "server.jar".into(), "nogui".into()]);
    Ok(ServerCommand {
        java,
        args,
        cwd: server.dir(paths),
        heap_mb: heap.max_mb,
    })
}

/// Run the server until it exits. Lines from `commands` go to its console. When `stop`
/// fires it gets a `stop` command (saving the world) and up to a minute before being killed.
pub async fn run(
    cmd: &ServerCommand,
    mut on_line: impl FnMut(String),
    mut commands: mpsc::UnboundedReceiver<String>,
    stop: impl std::future::Future<Output = ()>,
) -> Result<std::process::ExitStatus> {
    let mut command = tokio::process::Command::new(&cmd.java);
    command
        .args(&cmd.args)
        .current_dir(&cmd.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|e| other(format!("couldn't start Java: {e}")))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut stderr = BufReader::new(child.stderr.take().expect("piped stderr")).lines();
    let (mut out_done, mut err_done, mut stopping) = (false, false, false);
    tokio::pin!(stop);
    let deadline = tokio::time::sleep(Duration::from_secs(u64::MAX / 4));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            line = stdout.next_line(), if !out_done => match line? {
                Some(line) => on_line(strip_ansi(&line)),
                None => out_done = true,
            },
            line = stderr.next_line(), if !err_done => match line? {
                Some(line) => on_line(strip_ansi(&line)),
                None => err_done = true,
            },
            Some(command) = commands.next() => {
                let _ = stdin.write_all(format!("{}\n", command.trim()).as_bytes()).await;
                let _ = stdin.flush().await;
            }
            () = &mut stop, if !stopping => {
                stopping = true;
                let _ = stdin.write_all(b"stop\n").await;
                let _ = stdin.flush().await;
                deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(60));
            }
            () = &mut deadline => {
                on_line("netherNode: the server didn't stop within a minute; forcing it to close".into());
                let _ = child.kill().await;
            }
            status = child.wait() => return Ok(status?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> Paths {
        let root =
            std::env::temp_dir().join(format!("nethernode-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Paths::new(root)
    }

    #[test]
    fn create_writes_offline_mode_and_a_free_port() {
        let paths = temp_paths("create");
        let a = Server::create(&paths, "Survival", "1.21.1", ServerKind::Paper).unwrap();
        let b = Server::create(&paths, "Creative", "1.21.1", ServerKind::Vanilla).unwrap();
        assert_ne!(a.port, b.port);
        let props = properties(&a.dir(&paths));
        assert_eq!(props.get("online-mode").map(String::as_str), Some("false"));
        assert_eq!(props.get("server-port"), Some(&a.port.to_string()));
        assert!(Server::create(&paths, "Survival", "1.21.1", ServerKind::Paper).is_err());
        assert_eq!(Server::list(&paths).len(), 2);
        assert!(!eula_accepted(&paths, &a));
        accept_eula(&paths, &a).unwrap();
        assert!(eula_accepted(&paths, &a));
        std::fs::remove_dir_all(&paths.root).unwrap();
    }

    #[test]
    fn properties_keep_other_lines() {
        let paths = temp_paths("props");
        let dir = paths.root.join("s");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("server.properties"),
            "#comment\nmotd=old\nlevel-seed=42\n",
        )
        .unwrap();
        set_properties(&dir, &[("motd", "new"), ("pvp", "false")]).unwrap();
        let text = std::fs::read_to_string(dir.join("server.properties")).unwrap();
        assert_eq!(text, "#comment\nmotd=new\nlevel-seed=42\npvp=false\n");
        std::fs::remove_dir_all(&paths.root).unwrap();
    }

    #[test]
    fn strips_colour_codes() {
        assert_eq!(
            strip_ansi("\u{1b}[38;2;255;255;85mSteve joined the game\u{1b}[0m"),
            "Steve joined the game"
        );
        assert_eq!(strip_ansi("plain [INFO] text"), "plain [INFO] text");
    }

    #[test]
    fn whitelist_uses_offline_uuids() {
        let paths = temp_paths("wl");
        let s = Server::create(&paths, "W", "1.21.1", ServerKind::Vanilla).unwrap();
        set_whitelist(&paths, &s, true, &["Steve".into()]).unwrap();
        let json = std::fs::read_to_string(s.dir(&paths).join("whitelist.json")).unwrap();
        assert!(json.contains("5627dd98-e6be-3c21-b8a8-e92344183641"));
        assert_eq!(whitelist(&paths, &s), ["Steve"]);
        assert_eq!(
            properties(&s.dir(&paths))
                .get("white-list")
                .map(String::as_str),
            Some("true")
        );
        assert!(set_whitelist(&paths, &s, true, &["bad name!".into()]).is_err());
        std::fs::remove_dir_all(&paths.root).unwrap();
    }
}
