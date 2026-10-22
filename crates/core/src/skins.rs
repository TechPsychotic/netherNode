//! Custom skins and capes for offline accounts.
//!
//! The game normally asks Mojang what a player looks like; offline players are unknown
//! there, so everyone gets a default skin. netherNode runs a tiny skin service on
//! 127.0.0.1 and starts the game with authlib-injector, an open-source Java agent that sends
//! those lookups to it instead. Nothing logs in anywhere: the service only answers "what
//! does this player look like", for your accounts and for skins friends shared with you
//! through Invite friends. authlib-injector (AGPL-3.0) is downloaded from its official site
//! at a pinned version and checked against its SHA-256; it isn't bundled.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::account::{offline_uuid, validate_name};
use crate::download::{self, Download};
use crate::error::{Result, other};
use crate::paths::{Paths, ensure_parent};

pub const INJECTOR_VERSION: &str = "1.2.8";
const INJECTOR_URL: &str =
    "https://authlib-injector.yushi.moe/artifact/56/authlib-injector-1.2.8.jar";
const INJECTOR_SHA256: &str = "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10";
const MAX_PNG: usize = 256 * 1024;
/// The key every netherNode signs skin data with. Minecraft only shows another player's
/// skin if its signature checks out, and authlib-injector trusts a single key, so all
/// installs share this one. It's public on purpose: it only vouches for textures served from
/// 127.0.0.1, so a forged signature can at worst show a different local skin.
const SIGNING_KEY: &str = include_str!("skins-key.pem");
/// The skin service's usual port. Texture URLs contain it, and friends' games fetch the
/// same URLs from their own service, so it should be the same on every PC.
pub const PORT: u16 = 47_811;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    /// Steve-style arms (4 px).
    #[default]
    Classic,
    /// Alex-style arms (3 px).
    Slim,
}

/// A player's look: what's stored, and what's exchanged with friends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinRecord {
    pub name: String,
    pub model: Model,
    pub skin: Vec<u8>,
    pub cape: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    model: Model,
    skin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cape: Option<String>,
    /// One of this PC's accounts (friends' skins never overwrite these).
    own: bool,
}

/// Check a PNG is a real skin (64x64 or legacy 64x32) or cape (64x32, 22x17, 46x22) image.
pub fn validate_png(bytes: &[u8], cape: bool) -> Result<()> {
    if bytes.len() > MAX_PNG {
        return Err(other("that image is too large for a skin"));
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder
        .read_info()
        .map_err(|_| other("that file isn't a PNG image"))?;
    let (w, h) = (reader.info().width, reader.info().height);
    let ok = if cape {
        matches!((w, h), (64, 32) | (22, 17) | (46, 22))
    } else {
        matches!((w, h), (64, 64) | (64, 32))
    };
    if ok {
        Ok(())
    } else if cape {
        Err(other(format!("a cape must be 64×32 (this one is {w}×{h})")))
    } else {
        Err(other(format!(
            "a skin must be 64×64 or 64×32 (this one is {w}×{h})"
        )))
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Skins on disk: `<data>/skins/{index.json,textures/<sha256>.png}`.
#[derive(Debug, Clone)]
pub struct SkinStore {
    dir: PathBuf,
}

impl SkinStore {
    pub fn new(paths: &Paths) -> Self {
        Self {
            dir: paths.root.join("skins"),
        }
    }

    fn index_file(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    fn texture(&self, hash: &str) -> PathBuf {
        self.dir.join("textures").join(format!("{hash}.png"))
    }

    fn load(&self) -> BTreeMap<String, Entry> {
        std::fs::read(self.index_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, index: &BTreeMap<String, Entry>) -> Result<()> {
        let file = self.index_file();
        ensure_parent(&file)?;
        std::fs::write(file, serde_json::to_vec_pretty(index)?)?;
        Ok(())
    }

    fn put_texture(&self, bytes: &[u8]) -> Result<String> {
        let hash = hex_sha256(bytes);
        let path = self.texture(&hash);
        if !path.exists() {
            ensure_parent(&path)?;
            std::fs::write(path, bytes)?;
        }
        Ok(hash)
    }

    /// Set one of this PC's accounts' skin (and keep its cape).
    pub fn set_own_skin(&self, name: &str, png: &[u8], model: Model) -> Result<()> {
        validate_name(name)?;
        validate_png(png, false)?;
        let skin = self.put_texture(png)?;
        let mut index = self.load();
        let cape = index
            .get(name)
            .filter(|e| e.own)
            .and_then(|e| e.cape.clone());
        index.insert(
            name.to_string(),
            Entry {
                model,
                skin,
                cape,
                own: true,
            },
        );
        self.save(&index)
    }

    pub fn set_own_model(&self, name: &str, model: Model) -> Result<()> {
        let mut index = self.load();
        if let Some(entry) = index.get_mut(name).filter(|e| e.own) {
            entry.model = model;
            self.save(&index)?;
        }
        Ok(())
    }

    /// Set or clear the cape of an account that already has a skin.
    pub fn set_own_cape(&self, name: &str, png: Option<&[u8]>) -> Result<()> {
        let cape = match png {
            Some(png) => {
                validate_png(png, true)?;
                Some(self.put_texture(png)?)
            }
            None => None,
        };
        let mut index = self.load();
        let entry = index
            .get_mut(name)
            .filter(|e| e.own)
            .ok_or_else(|| other("choose a skin first, then add a cape"))?;
        entry.cape = cape;
        self.save(&index)
    }

    /// Back to the default Steve/Alex look.
    pub fn remove_own(&self, name: &str) -> Result<()> {
        let mut index = self.load();
        if index.get(name).is_some_and(|e| e.own) {
            index.remove(name);
            self.save(&index)?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<SkinRecord> {
        let entry = self.load().remove(name)?;
        let skin = std::fs::read(self.texture(&entry.skin)).ok()?;
        let cape = entry
            .cape
            .and_then(|h| std::fs::read(self.texture(&h)).ok());
        Some(SkinRecord {
            name: name.to_string(),
            model: entry.model,
            skin,
            cape,
        })
    }

    /// Store a skin a friend shared. Never replaces one of our own accounts.
    pub fn store_friend(&self, record: &SkinRecord) -> Result<()> {
        validate_name(&record.name)?;
        validate_png(&record.skin, false)?;
        if let Some(cape) = &record.cape {
            validate_png(cape, true)?;
        }
        let mut index = self.load();
        if index.get(&record.name).is_some_and(|e| e.own) {
            return Ok(());
        }
        let skin = self.put_texture(&record.skin)?;
        let cape = record
            .cape
            .as_deref()
            .map(|c| self.put_texture(c))
            .transpose()?;
        index.insert(
            record.name.clone(),
            Entry {
                model: record.model,
                skin,
                cape,
                own: false,
            },
        );
        self.save(&index)
    }

    /// Skins of this PC's own accounts (what we show friends).
    pub fn own_records(&self) -> Vec<SkinRecord> {
        let names: Vec<String> = self
            .load()
            .into_iter()
            .filter(|(_, e)| e.own)
            .map(|(n, _)| n)
            .collect();
        names.iter().filter_map(|n| self.get(n)).collect()
    }

    /// Find a player by the UUID the game uses (offline UUID, without dashes).
    fn by_uuid(&self, uuid: &str) -> Option<(String, Entry)> {
        let wanted = uuid.replace('-', "").to_lowercase();
        self.load()
            .into_iter()
            .find(|(name, _)| offline_uuid(name).replace('-', "") == wanted)
    }
}

/// authlib-injector's jar, downloaded once and checked against the pinned SHA-256.
pub async fn ensure_injector(http: &reqwest::Client, paths: &Paths) -> Result<PathBuf> {
    let jar = paths
        .root
        .join("cache")
        .join(format!("authlib-injector-{INJECTOR_VERSION}.jar"));
    let valid = |path: &Path| std::fs::read(path).is_ok_and(|b| hex_sha256(&b) == INJECTOR_SHA256);
    if valid(&jar) {
        return Ok(jar);
    }
    download::fetch(
        http,
        &Download {
            url: INJECTOR_URL.into(),
            dest: jar.clone(),
            sha1: None,
            size: None,
        },
    )
    .await?;
    if !valid(&jar) {
        let _ = std::fs::remove_file(&jar);
        return Err(other(
            "authlib-injector's download didn't match its checksum; skins are off for now",
        ));
    }
    Ok(jar)
}

/// JVM arguments that route the game's skin lookups to the local service on `port`.
pub fn agent_args(injector: &Path, port: u16) -> Vec<String> {
    vec![
        format!("-javaagent:{}=http://127.0.0.1:{port}/", injector.display()),
        // Don't show "netherNode skins" on the title screen.
        "-Dauthlibinjector.noShowServerName".into(),
    ]
}

/// Aborts the spawned skin service when dropped (when the game exits).
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Start the local skin service on a free port. Spawn the returned future (it runs forever).
pub async fn serve(
    paths: &Paths,
) -> Result<(u16, impl std::future::Future<Output = ()> + Send + 'static)> {
    let store = SkinStore::new(paths);
    let key = RsaPrivateKey::from_pkcs8_pem(SIGNING_KEY).map_err(|e| other(format!("key: {e}")))?;
    let public_pem = key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| other(format!("key: {e}")))?;
    let listener = match TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(listener) => listener,
        Err(_) if already_serving(&public_pem).await => {
            // Another game (or server) from this PC already runs it; share that one.
            return Ok((PORT, futures::future::Either::Left(std::future::pending())));
        }
        // Something else has the port: skins still work, just not friends' textures.
        Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    let port = listener.local_addr()?.port();
    let state = Arc::new(State {
        store,
        key,
        public_pem,
        port,
    });
    let run = futures::future::Either::Right(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let state = state.clone();
            tokio::spawn(async move {
                let _ = handle(stream, &state).await;
            });
        }
    });
    Ok((port, run))
}

struct State {
    store: SkinStore,
    key: RsaPrivateKey,
    public_pem: String,
    port: u16,
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn json(value: serde_json::Value) -> Self {
        Self {
            status: "200 OK",
            content_type: "application/json",
            body: value.to_string().into_bytes(),
        }
    }
    fn png(body: Vec<u8>) -> Self {
        Self {
            status: "200 OK",
            content_type: "image/png",
            body,
        }
    }
    fn no_content() -> Self {
        Self {
            status: "204 No Content",
            content_type: "text/plain",
            body: Vec::new(),
        }
    }
    fn not_found() -> Self {
        Self {
            status: "404 Not Found",
            content_type: "text/plain",
            body: b"not found".to_vec(),
        }
    }
}

/// Whether our own skin service (same key) is already listening on [`PORT`].
async fn already_serving(public_pem: &str) -> bool {
    let meta = async {
        let response = download::client()
            .get(format!("http://127.0.0.1:{PORT}/"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .ok()?;
        response.json::<serde_json::Value>().await.ok()
    };
    meta.await.is_some_and(|m| {
        m["meta"]["implementationName"] == "netherNode" && m["signaturePublickey"] == public_pem
    })
}

/// Read one HTTP/1.1 request (headers + Content-Length body), answer it, close.
async fn handle(mut stream: TcpStream, state: &State) -> Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let header_end = loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let (method, target) = (
        request_line.next().unwrap_or_default(),
        request_line.next().unwrap_or("/"),
    );
    let length = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(64 * 1024);
    let mut body = buf[header_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0u8; length - body.len()];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let response = route(state, method, path, query, &body);
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&response.body).await?;
    stream.shutdown().await?;
    Ok(())
}

fn route(state: &State, method: &str, path: &str, query: &str, body: &[u8]) -> Response {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        ("GET", [""]) => Response::json(serde_json::json!({
            "meta": {
                "serverName": "netherNode skins",
                "implementationName": "netherNode",
                "implementationVersion": env!("CARGO_PKG_VERSION"),
                // Never ask Mojang about anyone: everything is local.
                "feature.no_mojang_namespace": true,
                "feature.legacy_skin_api": true,
            },
            "skinDomains": ["127.0.0.1", "localhost"],
            "signaturePublickey": state.public_pem,
        })),
        ("GET", ["sessionserver", "session", "minecraft", "profile", uuid]) => {
            match state.store.by_uuid(uuid) {
                Some((name, entry)) => Response::json(profile(state, &name, &entry)),
                None => Response::no_content(),
            }
        }
        // A world opened to LAN (and online-mode servers) check that a joining player logged
        // in. Offline accounts can't, so everyone is let in, with their skin if we know it;
        // who may join is decided by the host approving friends in netherNode.
        ("GET", ["sessionserver", "session", "minecraft", "hasJoined"]) => {
            let name = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("username="))
                .unwrap_or_default();
            if validate_name(name).is_err() {
                return Response::no_content();
            }
            match state.store.load().remove(name) {
                Some(entry) => Response::json(profile(state, name, &entry)),
                None => Response::json(serde_json::json!({
                    "id": offline_uuid(name).replace('-', ""),
                    "name": name,
                    "properties": [],
                })),
            }
        }
        ("GET", ["api", "users", "profiles", "minecraft", name]) => match validate_name(name) {
            Ok(()) => Response::json(
                serde_json::json!({ "id": offline_uuid(name).replace('-', ""), "name": name }),
            ),
            Err(_) => Response::no_content(),
        },
        ("POST", ["api", "profiles", "minecraft"]) => {
            // Name -> profile lookups (player heads etc.): offline UUIDs, like the server uses.
            let names: Vec<String> = serde_json::from_slice(body).unwrap_or_default();
            let found: Vec<serde_json::Value> = names
                .iter()
                .filter(|n| validate_name(n).is_ok())
                .take(10)
                .map(|n| serde_json::json!({ "id": offline_uuid(n).replace('-', ""), "name": n }))
                .collect();
            Response::json(serde_json::Value::Array(found))
        }
        // 1.16.4+ asks what the player may do; without this, Multiplayer gets disabled.
        ("GET" | "POST", ["minecraftservices", "player", "attributes"]) => {
            Response::json(serde_json::json!({
                "privileges": {
                    "onlineChat": { "enabled": true },
                    "multiplayerServer": { "enabled": true },
                    "multiplayerRealms": { "enabled": false },
                    "telemetry": { "enabled": false },
                },
                "profanityFilterPreferences": { "profanityFilterOn": false },
            }))
        }
        ("GET", ["minecraftservices", "privacy", "blocklist"]) => {
            Response::json(serde_json::json!({ "blockedProfiles": [] }))
        }
        ("GET", ["textures", hash])
            if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            match std::fs::read(state.store.texture(hash)) {
                Ok(png) => Response::png(png),
                Err(_) => Response::not_found(),
            }
        }
        // Pre-1.7 versions fetch skins by name.
        ("GET", ["skins", "MinecraftSkins", file]) => legacy(state, file, false),
        ("GET", ["skins", "MinecraftCloaks", file]) => legacy(state, file, true),
        // Only used for online-mode servers, which offline accounts can't join anyway.
        ("POST", ["sessionserver", "session", "minecraft", "join"]) => Response::no_content(),
        _ => Response::not_found(),
    }
}

fn legacy(state: &State, file: &str, cape: bool) -> Response {
    let name = file.trim_end_matches(".png");
    let entry = state.store.load().remove(name);
    let hash = entry.and_then(|e| if cape { e.cape } else { Some(e.skin) });
    match hash.and_then(|h| std::fs::read(state.store.texture(&h)).ok()) {
        Some(png) => Response::png(png),
        None => Response::not_found(),
    }
}

/// A signed profile with texture URLs, as Mojang's session server would return it.
fn profile(state: &State, name: &str, entry: &Entry) -> serde_json::Value {
    let id = offline_uuid(name).replace('-', "");
    let url = |hash: &str| format!("http://127.0.0.1:{}/textures/{hash}", state.port);
    let mut textures = serde_json::Map::new();
    let mut skin = serde_json::json!({ "url": url(&entry.skin) });
    if entry.model == Model::Slim {
        skin["metadata"] = serde_json::json!({ "model": "slim" });
    }
    textures.insert("SKIN".into(), skin);
    if let Some(cape) = &entry.cape {
        textures.insert("CAPE".into(), serde_json::json!({ "url": url(cape) }));
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let value = B64.encode(
        serde_json::json!({ "timestamp": timestamp, "profileId": id, "profileName": name, "textures": textures })
            .to_string(),
    );
    let signature = sign(&state.key, value.as_bytes()).unwrap_or_default();
    serde_json::json!({
        "id": id,
        "name": name,
        "properties": [{ "name": "textures", "value": value, "signature": signature }],
    })
}

/// SHA1withRSA (PKCS#1 v1.5), base64: how Minecraft verifies texture properties.
fn sign(key: &RsaPrivateKey, data: &[u8]) -> Option<String> {
    // DER DigestInfo prefix for SHA-1.
    const SHA1_PREFIX: [u8; 15] = [
        0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14,
    ];
    let digest = Sha1::digest(data);
    let scheme = Pkcs1v15Sign {
        hash_len: Some(20),
        prefix: SHA1_PREFIX.to_vec().into_boxed_slice(),
    };
    key.sign(scheme, &digest).ok().map(|s| B64.encode(s))
}

/// Skins exchanged through Invite friends / Join friend, or `None` when skins are off.
pub fn for_sharing(paths: &Paths) -> Option<SkinStore> {
    crate::settings::Settings::load()
        .skins
        .then(|| SkinStore::new(paths))
}

/// Encode skins for a friend: a JSON array of [`WireSkin`].
pub(crate) fn encode(records: &[SkinRecord]) -> Vec<u8> {
    let wire: Vec<WireSkin> = records.iter().map(WireSkin::from).collect();
    serde_json::to_vec(&wire).unwrap_or_default()
}

/// Decode skins from a friend, dropping anything malformed.
pub(crate) fn decode(bytes: &[u8]) -> Vec<SkinRecord> {
    serde_json::from_slice::<Vec<WireSkin>>(bytes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(WireSkin::into_record)
        .collect()
}

/// Wire format for sharing skins with friends.
#[derive(Serialize, Deserialize)]
struct WireSkin {
    name: String,
    model: Model,
    skin: String,
    #[serde(default)]
    cape: Option<String>,
}

impl From<&SkinRecord> for WireSkin {
    fn from(r: &SkinRecord) -> Self {
        Self {
            name: r.name.clone(),
            model: r.model,
            skin: B64.encode(&r.skin),
            cape: r.cape.as_ref().map(|c| B64.encode(c)),
        }
    }
}

impl WireSkin {
    fn into_record(self) -> Option<SkinRecord> {
        let skin = B64.decode(self.skin).ok()?;
        let cape = match self.cape {
            Some(c) => Some(B64.decode(c).ok()?),
            None => None,
        };
        Some(SkinRecord {
            name: self.name,
            model: self.model,
            skin,
            cape,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer
                .write_image_data(&vec![120u8; (w * h * 4) as usize])
                .unwrap();
        }
        out
    }

    fn temp_paths(name: &str) -> Paths {
        let root =
            std::env::temp_dir().join(format!("nethernode-skins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Paths::new(root)
    }

    #[test]
    fn validates_skin_and_cape_sizes() {
        assert!(validate_png(&png(64, 64), false).is_ok());
        assert!(validate_png(&png(64, 32), false).is_ok());
        assert!(validate_png(&png(128, 128), false).is_err());
        assert!(validate_png(&png(64, 32), true).is_ok());
        assert!(validate_png(b"not a png", false).is_err());
    }

    #[test]
    fn friends_never_overwrite_own_accounts() {
        let paths = temp_paths("store");
        let store = SkinStore::new(&paths);
        store
            .set_own_skin("Steve", &png(64, 64), Model::Slim)
            .unwrap();
        let other = SkinRecord {
            name: "Steve".into(),
            model: Model::Classic,
            skin: png(64, 32),
            cape: None,
        };
        store.store_friend(&other).unwrap();
        assert_eq!(store.get("Steve").unwrap().model, Model::Slim);
        store
            .store_friend(&SkinRecord {
                name: "Alex".into(),
                ..other
            })
            .unwrap();
        assert_eq!(store.get("Alex").unwrap().skin, png(64, 32));
        std::fs::remove_dir_all(&paths.root).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_signed_profiles_the_game_can_verify() {
        use rsa::pkcs8::DecodePublicKey;
        let paths = temp_paths("serve");
        let store = SkinStore::new(&paths);
        store
            .set_own_skin("Steve", &png(64, 64), Model::Slim)
            .unwrap();
        let (port, run) = serve(&paths).await.unwrap();
        tokio::spawn(run);
        let http = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{port}");

        let meta: serde_json::Value = http
            .get(format!("{base}/"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let public =
            rsa::RsaPublicKey::from_public_key_pem(meta["signaturePublickey"].as_str().unwrap())
                .unwrap();

        let uuid = offline_uuid("Steve").replace('-', "");
        let profile: serde_json::Value = http
            .get(format!(
                "{base}/sessionserver/session/minecraft/profile/{uuid}?unsigned=false"
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let prop = &profile["properties"][0];
        let value = prop["value"].as_str().unwrap();
        let signature = B64.decode(prop["signature"].as_str().unwrap()).unwrap();
        // Verify exactly as Java's SHA1withRSA would.
        let digest = Sha1::digest(value.as_bytes());
        let scheme = Pkcs1v15Sign {
            hash_len: Some(20),
            prefix: vec![
                0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
                0x14,
            ]
            .into_boxed_slice(),
        };
        public.verify(scheme, &digest, &signature).unwrap();

        let textures: serde_json::Value =
            serde_json::from_slice(&B64.decode(value).unwrap()).unwrap();
        assert_eq!(textures["textures"]["SKIN"]["metadata"]["model"], "slim");
        let url = textures["textures"]["SKIN"]["url"].as_str().unwrap();
        let skin = http.get(url).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(skin.as_ref(), png(64, 64).as_slice());

        // Unknown players: no profile (the game falls back to the default skin).
        let unknown = http
            .get(format!(
                "{base}/sessionserver/session/minecraft/profile/{}",
                offline_uuid("Nobody").replace('-', "")
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 204);
        let attrs: serde_json::Value = http
            .get(format!("{base}/minecraftservices/player/attributes"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(attrs["privileges"]["multiplayerServer"]["enabled"], true);

        // LAN worlds: anyone is let in, keeping their offline UUID, with their skin.
        let joined: serde_json::Value = http
            .get(format!(
                "{base}/sessionserver/session/minecraft/hasJoined?username=Steve&serverId=abc"
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(joined["id"], uuid);
        assert_eq!(joined["properties"][0]["name"], "textures");
        let stranger: serde_json::Value = http
            .get(format!(
                "{base}/sessionserver/session/minecraft/hasJoined?username=Nobody&serverId=abc"
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(stranger["name"], "Nobody");
        std::fs::remove_dir_all(&paths.root).unwrap();
    }
}
