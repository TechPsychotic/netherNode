//! Play a LAN world with friends over the internet, with no port forwarding.
//!
//! Built on iroh: QUIC with TLS 1.3, peers identified by public key. It punches a direct
//! path through NAT when it can and otherwise falls back to a relay over HTTPS (port 443),
//! which only ever sees encrypted packets.
//!
//! - The **host** opens the world to LAN in Minecraft and shares a join code: their
//!   endpoint id plus a random secret. Each friend's first connection must present the
//!   secret, then the host approves it. Only the Minecraft LAN port is reachable.
//! - The **joiner** gets a local port that tunnels to the host. It's announced on their own
//!   LAN so the world shows up in Minecraft's LAN list, and only accepts connections from
//!   the same computer.
//!
//! Each Minecraft TCP connection (server-list ping, game) is one QUIC stream. Skins travel
//! on one more stream: the joiner sends theirs, the host answers with everyone's and passes
//! on new ones as friends arrive, so everybody sees everybody's skin.

use std::collections::HashSet;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use crate::error::{Result, other};
use crate::modsync::Manifest;
use crate::skins::{self, SkinRecord, SkinStore};

pub const ALPN: &[u8] = b"nethernode/minecraft-lan/3";
/// First byte of every stream after approval: what the stream carries.
const STREAM_MINECRAFT: u8 = 1;
const STREAM_SKINS: u8 = 2;
/// Followed by the host's port (u16): a TCP connection to an extra port.
const STREAM_TCP_PORT: u8 = 3;
/// Followed by a mod's SHA-1 (40 hex characters): send that mod jar.
const STREAM_MOD_FILE: u8 = 4;
/// UDP packets too big for one QUIC datagram, both ways (see [`UdpLane`]).
const STREAM_UDP_LANE: u8 = 5;
/// Largest skin message accepted (a handful of skins and capes).
const MAX_SKIN_FRAME: usize = 4 * 1024 * 1024;
/// Most skins one friend may announce.
const MAX_FRIEND_SKINS: usize = 8;
const SECRET_LEN: usize = 8;
/// Minecraft's LAN discovery: multicast group and port the game listens on.
const LAN_GROUP: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(224, 0, 2, 60)), 4445);

/// What a friend needs to join: the host's id and the session secret.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinCode {
    pub host: EndpointId,
    secret: [u8; SECRET_LEN],
}

impl fmt::Display for JoinCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.host.to_z32(), hex::encode(self.secret))
    }
}

impl fmt::Debug for JoinCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the secret by accident.
        write!(f, "JoinCode({}…)", self.host.fmt_short())
    }
}

impl FromStr for JoinCode {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self> {
        let bad = || other("that doesn't look like a netherNode join code");
        let (id, secret) = s.trim().rsplit_once('-').ok_or_else(bad)?;
        let host = EndpointId::from_z32(id).map_err(|_| bad())?;
        let bytes = hex::decode(secret).map_err(|_| bad())?;
        Ok(Self {
            host,
            secret: bytes.try_into().map_err(|_| bad())?,
        })
    }
}

/// Copy both directions between a local TCP connection and a QUIC stream until both close.
async fn pipe(tcp: TcpStream, mut send: SendStream, mut recv: RecvStream) {
    let _ = tcp.set_nodelay(true); // Minecraft sends many small packets
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    let up = async {
        let _ = tokio::io::copy(&mut tcp_read, &mut send).await;
        let _ = send.finish();
    };
    let down = async {
        let _ = tokio::io::copy(&mut recv, &mut tcp_write).await;
        let _ = tcp_write.shutdown().await;
    };
    tokio::join!(up, down);
}

async fn bind(identity: Option<iroh::SecretKey>) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0).alpns(vec![ALPN.to_vec()]);
    if let Some(key) = identity {
        builder = builder.secret_key(key);
    }
    // Diagnostics: force the relay path, as on networks where hole punching fails.
    if std::env::var_os("NETHERNODE_SHARE_RELAY_ONLY").is_some() {
        builder = builder.clear_ip_transports();
    }
    builder
        .bind()
        .await
        .map_err(|e| other(format!("couldn't start networking: {e}")))
}

pub(crate) async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> std::io::Result<()> {
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    Ok(send.write_all(bytes).await?)
}

pub(crate) async fn read_frame(recv: &mut RecvStream) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.ok()?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_SKIN_FRAME {
        return None;
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await.ok()?;
    Some(buf)
}

/// Skins known in one hosting session, and a channel announcing new ones.
struct SkinHub {
    store: SkinStore,
    known: Mutex<std::collections::BTreeMap<String, SkinRecord>>,
    news: tokio::sync::broadcast::Sender<Arc<Vec<SkinRecord>>>,
}

impl SkinHub {
    fn new(store: SkinStore) -> Self {
        let known = store
            .own_records()
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();
        Self {
            store,
            known: Mutex::new(known),
            news: tokio::sync::broadcast::channel(16).0,
        }
    }

    /// A friend's skin stream: take theirs, send everyone's, then keep them up to date.
    async fn serve(&self, mut send: SendStream, mut recv: RecvStream) {
        let mut news = self.news.subscribe();
        let theirs: Vec<SkinRecord> = match read_frame(&mut recv).await {
            Some(frame) => skins::decode(&frame)
                .into_iter()
                .take(MAX_FRIEND_SKINS)
                .collect(),
            None => return,
        };
        // Only well-formed skins that don't clash with the host's own accounts are kept.
        let theirs: Vec<SkinRecord> = theirs
            .into_iter()
            .filter(|r| self.store.store_friend(r).is_ok())
            .collect();
        let everyone: Vec<SkinRecord> = {
            let mut known = self.known.lock().unwrap();
            for r in &theirs {
                known.entry(r.name.clone()).or_insert_with(|| r.clone());
            }
            known.values().cloned().collect()
        };
        if !theirs.is_empty() {
            let _ = self.news.send(Arc::new(theirs.clone()));
        }
        if write_frame(&mut send, &skins::encode(&everyone))
            .await
            .is_err()
        {
            return;
        }
        let mine: HashSet<String> = theirs.into_iter().map(|r| r.name).collect();
        loop {
            match news.recv().await {
                Ok(records) => {
                    let records: Vec<SkinRecord> = records
                        .iter()
                        .filter(|r| !mine.contains(&r.name))
                        .cloned()
                        .collect();
                    if !records.is_empty()
                        && write_frame(&mut send, &skins::encode(&records))
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
    }
}

/// Joiner side: send our skins, then store whatever the host sends until the session ends.
async fn exchange_skins(conn: Connection, store: SkinStore) {
    let Ok((mut send, mut recv)) = conn.open_bi().await else {
        return;
    };
    let mine = skins::encode(&store.own_records());
    if send.write_all(&[STREAM_SKINS]).await.is_err()
        || write_frame(&mut send, &mine).await.is_err()
    {
        return;
    }
    while let Some(frame) = read_frame(&mut recv).await {
        for record in skins::decode(&frame) {
            let _ = store.store_friend(&record);
        }
    }
}

/// A port besides Minecraft's that mods use, such as Simple Voice Chat's UDP port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExtraPort {
    pub udp: bool,
    pub port: u16,
}

impl fmt::Display for ExtraPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", if self.udp { "UDP" } else { "TCP" }, self.port)
    }
}

/// What an approved friend is told about the session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Welcome {
    /// The world's or server's name.
    title: String,
    /// The host's Minecraft port (UDP on the same number is forwarded too, for mods that use
    /// it, like Simple Voice Chat by default).
    game_port: u16,
    #[serde(default)]
    extra_ports: Vec<ExtraPort>,
    #[serde(default)]
    manifest: Option<Manifest>,
}

/// What to share and how.
#[derive(Debug, Clone, Default)]
pub struct HostOptions {
    /// The Minecraft port on this computer (a world opened to LAN, or a server).
    pub lan_port: u16,
    /// Shown to friends.
    pub title: String,
    pub extra_ports: Vec<ExtraPort>,
    /// The mods friends need, and the folder they can be fetched from.
    pub manifest: Option<(Manifest, PathBuf)>,
    /// Exchange skins (see [`crate::skins::for_sharing`]).
    pub skins: Option<SkinStore>,
}

/// Something the host's UI should show.
pub enum HostEvent {
    /// Sharing is live; give friends the short code (or the full one if it couldn't be
    /// published).
    Ready {
        code: JoinCode,
        short: Option<crate::invite::ShortCode>,
    },
    /// Someone with the right code wants in. Answer through `reply` (dropping it = deny).
    /// `id` identifies their netherNode, so friends can be let in automatically.
    Request {
        friend: String,
        id: EndpointId,
        reply: oneshot::Sender<bool>,
    },
    Joined {
        friend: String,
    },
    Left {
        friend: String,
    },
}

/// Share a Minecraft port until `stop` completes.
pub async fn host(
    options: HostOptions,
    on_event: impl Fn(HostEvent) + Send + Sync + 'static,
    stop: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let endpoint = bind(None).await?;
    // Wait until reachable via a relay, so the code works straight away.
    let _ = tokio::time::timeout(Duration::from_secs(20), endpoint.online()).await;
    let mut secret = [0u8; SECRET_LEN];
    getrandom::fill(&mut secret).map_err(|e| other(format!("no secure random numbers: {e}")))?;
    let code = JoinCode {
        host: endpoint.id(),
        secret,
    };
    let http = crate::download::client();
    let short = match crate::invite::ShortCode::random() {
        Ok(short) => match crate::invite::publish(&http, &short, &code).await {
            Ok(()) => Some(short),
            Err(e) => {
                eprintln!("{e}; friends can use the full code");
                None
            }
        },
        Err(_) => None,
    };
    let republish = {
        let (short, code) = (short.clone(), code.clone());
        async move {
            let Some(short) = short else {
                return std::future::pending().await;
            };
            loop {
                tokio::time::sleep(crate::invite::REPUBLISH).await;
                let _ = crate::invite::publish(&http, &short, &code).await;
            }
        }
    };
    on_event(HostEvent::Ready { code, short });

    let session = Arc::new(Session {
        welcome: Welcome {
            title: options.title.clone(),
            game_port: options.lan_port,
            extra_ports: options.extra_ports.clone(),
            manifest: options.manifest.as_ref().map(|(m, _)| m.clone()),
        },
        mods_dir: options.manifest.map(|(_, dir)| dir),
        secret,
        on_event: Box::new(on_event),
        approved: Mutex::default(),
        hub: options.skins.map(SkinHub::new),
    });
    tokio::pin!(stop, republish);
    loop {
        tokio::select! {
            () = &mut republish => {}
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let session = session.clone();
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else { return };
                    session.serve_friend(conn).await;
                });
            }
            () = &mut stop => break,
        }
    }
    endpoint.close().await;
    Ok(())
}

/// One hosting session, shared by every friend's connection.
struct Session {
    welcome: Welcome,
    mods_dir: Option<PathBuf>,
    secret: [u8; SECRET_LEN],
    on_event: Box<dyn Fn(HostEvent) + Send + Sync>,
    approved: Mutex<HashSet<EndpointId>>,
    hub: Option<SkinHub>,
}

impl Session {
    fn allowed_port(&self, udp: bool, port: u16) -> bool {
        (udp && port == self.welcome.game_port)
            || self.welcome.extra_ports.contains(&ExtraPort { udp, port })
    }

    async fn serve_friend(self: Arc<Self>, conn: Connection) {
        let friend_id = conn.remote_id();
        let friend = friend_id.fmt_short().to_string();

        // 1. The secret, on a control stream. Wrong or missing: drop without a prompt.
        let Ok(Ok((mut control_send, mut control_recv))) =
            tokio::time::timeout(Duration::from_secs(15), conn.accept_bi()).await
        else {
            return;
        };
        let mut presented = [0u8; SECRET_LEN];
        let read = tokio::time::timeout(
            Duration::from_secs(15),
            control_recv.read_exact(&mut presented),
        )
        .await;
        if !matches!(read, Ok(Ok(_))) || presented != self.secret {
            conn.close(1u32.into(), b"wrong code");
            return;
        }

        // 2. The host decides (once per friend per session).
        let known = self.approved.lock().unwrap().contains(&friend_id);
        if !known {
            let (reply, answer) = oneshot::channel();
            (self.on_event)(HostEvent::Request {
                friend: friend.clone(),
                id: friend_id,
                reply,
            });
            let allowed = matches!(
                tokio::time::timeout(Duration::from_secs(120), answer).await,
                Ok(Ok(true))
            );
            if !allowed {
                let _ = control_send.write_all(&[0]).await;
                let _ = control_send.finish();
                conn.close(2u32.into(), b"denied");
                return;
            }
            self.approved.lock().unwrap().insert(friend_id);
        }
        let welcome = serde_json::to_vec(&self.welcome).unwrap_or_default();
        if control_send.write_all(&[1]).await.is_err()
            || write_frame(&mut control_send, &welcome).await.is_err()
        {
            return;
        }
        let _ = control_send.finish();
        (self.on_event)(HostEvent::Joined {
            friend: friend.clone(),
        });

        // 3. Datagrams carry UDP for the game port and extra UDP ports.
        let udp = UdpRelay::default();
        let datagrams = {
            let (conn, session, udp) = (conn.clone(), self.clone(), udp.clone());
            async move {
                while let Ok(datagram) = conn.read_datagram().await {
                    let Some((port, payload)) = split_datagram(&datagram) else {
                        continue;
                    };
                    if session.allowed_port(true, port) {
                        udp.host_send(&conn, port, payload).await;
                    }
                }
            }
        };

        // 4. Every stream is a Minecraft connection, an extra TCP port, skins, a mod file or
        // the lane for big UDP packets.
        let streams = async {
            while let Ok((send, mut recv)) = conn.accept_bi().await {
                let session = self.clone();
                let (udp, conn) = (udp.clone(), conn.clone());
                tokio::spawn(async move {
                    let mut kind = [0u8; 1];
                    if recv.read_exact(&mut kind).await.is_err() {
                        return;
                    }
                    match kind[0] {
                        STREAM_MINECRAFT => {
                            let port = session.welcome.game_port;
                            if let Ok(tcp) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
                                pipe(tcp, send, recv).await;
                            }
                        }
                        STREAM_TCP_PORT => {
                            let mut port = [0u8; 2];
                            if recv.read_exact(&mut port).await.is_err() {
                                return;
                            }
                            let port = u16::from_be_bytes(port);
                            if !session.allowed_port(false, port) {
                                return;
                            }
                            if let Ok(tcp) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
                                pipe(tcp, send, recv).await;
                            }
                        }
                        STREAM_SKINS => {
                            if let Some(hub) = &session.hub {
                                hub.serve(send, recv).await;
                            }
                        }
                        STREAM_MOD_FILE => session.send_mod(send, recv).await,
                        STREAM_UDP_LANE => {
                            udp.lane.set(send).await;
                            while let Some((port, payload)) = read_lane_frame(&mut recv).await {
                                if session.allowed_port(true, port) {
                                    udp.host_send(&conn, port, &payload).await;
                                }
                            }
                        }
                        _ => {}
                    }
                });
            }
        };
        tokio::select! {
            () = datagrams => {}
            () = streams => {}
        }
        (self.on_event)(HostEvent::Left { friend });
    }

    /// Send one mod jar from the manifest: `u64` length, then the bytes (`u64::MAX` = no).
    async fn send_mod(&self, mut send: SendStream, mut recv: RecvStream) {
        let mut sha1 = [0u8; 40];
        if recv.read_exact(&mut sha1).await.is_err() {
            return;
        }
        let sha1 = String::from_utf8_lossy(&sha1).to_string();
        let file = match (&self.welcome.manifest, &self.mods_dir) {
            (Some(manifest), Some(dir)) => crate::modsync::file_for(manifest, dir, &sha1),
            _ => None,
        };
        let bytes = match file {
            Some(path) => tokio::fs::read(path).await.ok(),
            None => None,
        };
        let _ = match bytes {
            Some(bytes) => {
                let _ = send.write_all(&(bytes.len() as u64).to_be_bytes()).await;
                send.write_all(&bytes).await
            }
            None => send.write_all(&u64::MAX.to_be_bytes()).await,
        };
        let _ = send.finish();
        // Let the friend read everything before the stream goes away.
        let _ = tokio::time::timeout(Duration::from_secs(600), send.stopped()).await;
    }
}

/// Where the host's mod ports are reached: this computer. Diagnostics can point it at
/// another loopback address (`NETHERNODE_SHARE_HOST_TARGET=127.0.0.2`) to test host and
/// joiner on one machine without their ports colliding.
fn host_target() -> Ipv4Addr {
    std::env::var("NETHERNODE_SHARE_HOST_TARGET")
        .ok()
        .and_then(|a| a.parse().ok())
        .unwrap_or(Ipv4Addr::LOCALHOST)
}

/// Where the joiner opens extra ports: every interface, so LAN-list joins (which use this
/// computer's LAN address) work. Diagnostics: `NETHERNODE_SHARE_JOIN_BIND=::1` (or an IPv4 address).
fn joiner_bind() -> Option<std::net::IpAddr> {
    std::env::var("NETHERNODE_SHARE_JOIN_BIND")
        .ok()
        .and_then(|a| a.parse().ok())
}

/// A datagram is `[port: u16 big-endian][payload]`, `port` being the host's UDP port.
fn split_datagram(datagram: &[u8]) -> Option<(u16, &[u8])> {
    let (port, payload) = datagram.split_at_checked(2)?;
    Some((u16::from_be_bytes([port[0], port[1]]), payload))
}

fn make_datagram(port: u16, payload: &[u8]) -> bytes::Bytes {
    let mut out = Vec::with_capacity(payload.len() + 2);
    out.extend_from_slice(&port.to_be_bytes());
    out.extend_from_slice(payload);
    out.into()
}

/// Host side of UDP forwarding: one local socket per port per friend, so replies find their
/// way back to the right friend.
#[derive(Clone, Default)]
struct UdpRelay {
    sockets: Arc<tokio::sync::Mutex<std::collections::HashMap<u16, Arc<UdpSocket>>>>,
    lane: UdpLane,
}

/// UDP packets go as QUIC datagrams, which are limited to about 1,200 bytes (less through a
/// relay). Bigger ones (Simple Voice Chat allows 1,275) would be dropped, so they travel on
/// this reliable stream instead, as frames of `[port: u16][length: u16][payload]`.
#[derive(Clone, Default)]
struct UdpLane(Arc<tokio::sync::Mutex<Option<SendStream>>>);

impl UdpLane {
    async fn set(&self, send: SendStream) {
        *self.0.lock().await = Some(send);
    }

    /// Send one UDP packet to the other side, as a datagram when it fits.
    async fn send(&self, conn: &Connection, port: u16, payload: &[u8]) {
        let datagram = make_datagram(port, payload);
        let fits = conn
            .max_datagram_size()
            .is_some_and(|max| datagram.len() <= max);
        if fits && conn.send_datagram(datagram).is_ok() {
            return;
        }
        let Ok(len) = u16::try_from(payload.len()) else {
            return;
        };
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&port.to_be_bytes());
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(payload);
        let mut lane = self.0.lock().await;
        if let Some(send) = lane.as_mut()
            && send.write_all(&frame).await.is_err()
        {
            *lane = None;
        }
    }
}

/// Read one `[port][length][payload]` frame from a UDP lane.
async fn read_lane_frame(recv: &mut RecvStream) -> Option<(u16, Vec<u8>)> {
    let mut header = [0u8; 4];
    recv.read_exact(&mut header).await.ok()?;
    let port = u16::from_be_bytes([header[0], header[1]]);
    let mut payload = vec![0u8; usize::from(u16::from_be_bytes([header[2], header[3]]))];
    recv.read_exact(&mut payload).await.ok()?;
    Some((port, payload))
}

impl UdpRelay {
    async fn host_send(&self, conn: &Connection, port: u16, payload: &[u8]) {
        let socket = {
            let mut sockets = self.sockets.lock().await;
            match sockets.get(&port) {
                Some(s) => s.clone(),
                None => {
                    let Ok(socket) = UdpSocket::bind((host_target(), 0)).await else {
                        return;
                    };
                    if socket.connect((host_target(), port)).await.is_err() {
                        return;
                    }
                    let socket = Arc::new(socket);
                    sockets.insert(port, socket.clone());
                    // Replies from the mod go back to this friend.
                    let (reply, conn, lane) = (socket.clone(), conn.clone(), self.lane.clone());
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 65_536];
                        while let Ok(n) = reply.recv(&mut buf).await {
                            if conn.close_reason().is_some() {
                                break;
                            }
                            lane.send(&conn, port, &buf[..n]).await;
                        }
                    });
                    socket
                }
            }
        };
        let _ = socket.send(payload).await;
    }
}

/// Something the joiner's UI should show.
#[derive(Debug, Clone)]
pub enum JoinEvent {
    Connecting,
    /// Connected; waiting for the host to click Allow.
    WaitingForApproval,
    /// In: Minecraft can connect to `localhost:port` (it's also in the LAN list).
    Ready {
        port: u16,
        title: String,
        /// Extra ports that couldn't be opened here (another program uses them).
        blocked: Vec<ExtraPort>,
    },
    /// The host plays with mods; [`ModSource`] fetches the ones Modrinth doesn't have.
    Mods(Manifest, ModSource),
}

/// Fetches mod jars from the host during a session.
#[derive(Clone)]
pub struct ModSource(Connection);

impl fmt::Debug for ModSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ModSource")
    }
}

impl ModSource {
    pub async fn fetch(&self, sha1: String) -> Result<Vec<u8>> {
        let failed = |e: String| other(format!("couldn't get a mod from your friend: {e}"));
        let (mut send, mut recv) = self.0.open_bi().await.map_err(|e| failed(e.to_string()))?;
        let mut request = vec![STREAM_MOD_FILE];
        request.extend_from_slice(sha1.as_bytes());
        send.write_all(&request)
            .await
            .map_err(|e| failed(e.to_string()))?;
        let _ = send.finish();
        let mut len = [0u8; 8];
        recv.read_exact(&mut len)
            .await
            .map_err(|e| failed(e.to_string()))?;
        let len = u64::from_be_bytes(len);
        if len > crate::modsync::MAX_MOD_SIZE {
            return Err(failed("they don't have it any more".into()));
        }
        let mut bytes = vec![0u8; len as usize];
        recv.read_exact(&mut bytes)
            .await
            .map_err(|e| failed(e.to_string()))?;
        Ok(bytes)
    }
}

/// Join a friend's world until `stop` completes or the host ends it. With `skins`, our
/// skins go to the host and everyone's come back. `via` is this computer's friends endpoint
/// (see [`crate::friends`]): joining through it lets a friend's netherNode recognise us and
/// let us in without asking. `None` joins anonymously.
pub async fn join(
    code: &JoinCode,
    name: &str,
    skins: Option<SkinStore>,
    via: Option<Endpoint>,
    on_event: impl Fn(JoinEvent) + Send + Sync + 'static,
    stop: impl std::future::Future<Output = ()>,
) -> Result<()> {
    on_event(JoinEvent::Connecting);
    let owned = via.is_none();
    let endpoint = match via {
        Some(endpoint) => endpoint,
        None => bind(None).await?,
    };
    let conn = tokio::time::timeout(Duration::from_secs(45), endpoint.connect(code.host, ALPN))
        .await
        .map_err(|_| {
            other("couldn't reach your friend; check the code and that they're still sharing")
        })?
        .map_err(|e| other(format!("couldn't reach your friend: {e}")))?;

    let (mut control_send, mut control_recv) = conn
        .open_bi()
        .await
        .map_err(|e| other(format!("connection failed: {e}")))?;
    control_send
        .write_all(&code.secret)
        .await
        .map_err(|e| other(format!("connection failed: {e}")))?;
    on_event(JoinEvent::WaitingForApproval);
    let mut answer = [0u8; 1];
    match tokio::time::timeout(
        Duration::from_secs(130),
        control_recv.read_exact(&mut answer),
    )
    .await
    {
        Ok(Ok(_)) if answer[0] == 1 => {}
        Ok(Ok(_)) => return Err(other("your friend declined")),
        Ok(Err(_)) => return Err(other("your friend declined, or the code is wrong")),
        Err(_) => return Err(other("your friend didn't answer in time")),
    }
    let welcome: Welcome = read_frame(&mut control_recv)
        .await
        .and_then(|f| serde_json::from_slice(&f).ok())
        .unwrap_or_default();

    let skin_sync = match skins {
        Some(store) => tokio::spawn(exchange_skins(conn.clone(), store)),
        None => tokio::spawn(async {}),
    };
    // Listen on all interfaces so the LAN-list entry (which uses this computer's LAN address)
    // works, but only accept connections coming from this same computer.
    let listener = listen_tcp(0)?;
    let port = listener.local_addr()?.port();

    // UDP on the game port's number (as the host has it) plus extra ports.
    let mut blocked = Vec::new();
    let mut udp: std::collections::HashMap<u16, Arc<JoinerUdp>> = Default::default();
    if let Ok(socket) = listen_udp(port) {
        udp.insert(welcome.game_port, Arc::new(JoinerUdp::new(socket)));
    }
    let mut tcp_extra = Vec::new();
    for extra in &welcome.extra_ports {
        if extra.udp {
            let socket = match joiner_bind() {
                Some(ip) => UdpSocket::bind((ip, extra.port)).await,
                None => listen_udp(extra.port),
            };
            match socket {
                Ok(socket) => {
                    udp.insert(extra.port, Arc::new(JoinerUdp::new(socket)));
                }
                Err(_) => blocked.push(*extra),
            }
        } else {
            let listener = match joiner_bind() {
                Some(ip) => TcpListener::bind((ip, extra.port)).await,
                None => listen_tcp(extra.port),
            };
            match listener {
                Ok(listener) => tcp_extra.push((extra.port, listener)),
                Err(_) => blocked.push(*extra),
            }
        }
    }
    let udp = Arc::new(udp);
    on_event(JoinEvent::Ready {
        port,
        title: welcome.title.clone(),
        blocked,
    });
    if let Some(manifest) = welcome.manifest.clone().filter(Manifest::is_modded) {
        on_event(JoinEvent::Mods(manifest, ModSource(conn.clone())));
    }

    let mut tasks = Vec::new();
    // The lane for UDP packets too big for a datagram, both ways.
    let lane = UdpLane::default();
    if !udp.is_empty()
        && let Ok((mut send, mut recv)) = conn.open_bi().await
        && send.write_all(&[STREAM_UDP_LANE]).await.is_ok()
    {
        lane.set(send).await;
        let udp = udp.clone();
        tasks.push(tokio::spawn(async move {
            while let Some((port, payload)) = read_lane_frame(&mut recv).await {
                if let Some(joiner) = udp.get(&port) {
                    joiner.downlink(&payload).await;
                }
            }
        }));
    }
    for (host_port, joiner) in udp.iter() {
        let (joiner, conn, lane, host_port) =
            (joiner.clone(), conn.clone(), lane.clone(), *host_port);
        tasks.push(tokio::spawn(async move {
            joiner.uplink(&conn, &lane, host_port).await
        }));
    }
    {
        let (udp, conn) = (udp.clone(), conn.clone());
        tasks.push(tokio::spawn(async move {
            while let Ok(datagram) = conn.read_datagram().await {
                if let Some((port, payload)) = split_datagram(&datagram)
                    && let Some(joiner) = udp.get(&port)
                {
                    joiner.downlink(payload).await;
                }
            }
        }));
    }
    for (extra_port, listener) in tcp_extra {
        let conn = conn.clone();
        tasks.push(tokio::spawn(async move {
            while let Ok((tcp, peer)) = listener.accept().await {
                if !is_this_computer(&tcp, peer) {
                    continue;
                }
                let conn = conn.clone();
                tokio::spawn(async move {
                    let mut header = vec![STREAM_TCP_PORT];
                    header.extend_from_slice(&extra_port.to_be_bytes());
                    if let Ok((mut send, recv)) = conn.open_bi().await
                        && send.write_all(&header).await.is_ok()
                    {
                        pipe(tcp, send, recv).await;
                    }
                });
            }
        }));
    }

    let lan_name = format!("{} (netherNode)", welcome.title.trim());
    let lan_name = if welcome.title.trim().is_empty() {
        name.to_string()
    } else {
        lan_name
    };
    let announce = announce_on_lan(&lan_name, port);
    tokio::pin!(stop, announce);
    let result = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((tcp, peer)) = accepted else { continue };
                if !is_this_computer(&tcp, peer) {
                    continue; // someone else on the LAN; this tunnel is only for us
                }
                let conn = conn.clone();
                tokio::spawn(async move {
                    if let Ok((mut send, recv)) = conn.open_bi().await
                        && send.write_all(&[STREAM_MINECRAFT]).await.is_ok()
                    {
                        pipe(tcp, send, recv).await;
                    }
                });
            }
            reason = conn.closed() => break Err(other(format!("the host ended the session ({reason})"))),
            () = &mut announce => {}
            () = &mut stop => break Ok(()),
        }
    };
    skin_sync.abort();
    for task in tasks {
        task.abort();
    }
    conn.close(0u32.into(), b"bye");
    if owned {
        endpoint.close().await;
    }
    result
}

fn is_this_computer(tcp: &TcpStream, peer: SocketAddr) -> bool {
    let local = tcp.local_addr().map(|a| a.ip().to_canonical()).ok();
    let peer = peer.ip().to_canonical();
    peer.is_loopback() || Some(peer) == local
}

/// A TCP listener on every interface, IPv6 and IPv4 alike: `localhost` is `::1` on many
/// systems (Windows, Fedora), and Minecraft tries that first.
fn listen_tcp(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let dual = || -> std::io::Result<std::net::TcpListener> {
        let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_only_v6(false)?;
        socket.bind(&SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port)).into())?;
        socket.listen(128)?;
        Ok(socket.into())
    };
    // Without IPv6 on this computer, IPv4 alone.
    let listener =
        dual().or_else(|_| std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)))?;
    listener.set_nonblocking(true)?;
    TcpListener::from_std(listener)
}

/// A UDP socket on every interface, IPv6 and IPv4 alike (see [`listen_tcp`]).
fn listen_udp(port: u16) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let dual = || -> std::io::Result<std::net::UdpSocket> {
        let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_only_v6(false)?;
        socket.bind(&SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, port)).into())?;
        Ok(socket.into())
    };
    let socket = dual().or_else(|_| std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)))?;
    socket.set_nonblocking(true)?;
    UdpSocket::from_std(socket)
}

/// Joiner side of one forwarded UDP port: packets from the game (or a mod) on this computer
/// go to the host; replies come back to whoever sent last.
struct JoinerUdp {
    socket: UdpSocket,
    peer: tokio::sync::Mutex<Option<SocketAddr>>,
}

impl JoinerUdp {
    fn new(socket: UdpSocket) -> Self {
        Self {
            socket,
            peer: tokio::sync::Mutex::new(None),
        }
    }

    async fn uplink(&self, conn: &Connection, lane: &UdpLane, host_port: u16) {
        let mut buf = vec![0u8; 65_536];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf).await {
            let ip = from.ip().to_canonical();
            if !ip.is_loopback() && !is_local_ip(ip) {
                continue;
            }
            *self.peer.lock().await = Some(from);
            lane.send(conn, host_port, &buf[..n]).await;
        }
    }

    async fn downlink(&self, payload: &[u8]) {
        if let Some(peer) = *self.peer.lock().await {
            let _ = self.socket.send_to(payload, peer).await;
        }
    }
}

/// Whether `ip` belongs to this computer (a UDP sender has no "local address" to compare).
fn is_local_ip(ip: std::net::IpAddr) -> bool {
    std::net::UdpSocket::bind((ip.to_canonical(), 0)).is_ok()
}

/// Ports that mods in this game or server folder listen on besides Minecraft's, read from
/// their configs. Today: Simple Voice Chat (UDP, 24454 by default; `-1` means it shares the
/// game port, which is always forwarded anyway).
pub fn detect_mod_ports(dir: &std::path::Path, game_port: u16) -> Vec<ExtraPort> {
    let mut ports = Vec::new();
    let has_mod = |needle: &str| {
        std::fs::read_dir(dir.join("mods"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(needle)
            })
    };
    let config = dir.join("config/voicechat/voicechat-server.properties");
    let configured = std::fs::read_to_string(&config).ok().and_then(|text| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == "port")
            .and_then(|(_, v)| v.trim().parse::<i32>().ok())
    });
    let voice = match configured {
        Some(port) if port > 0 => u16::try_from(port).ok(),
        Some(_) => None, // same as the game port
        None if has_mod("voicechat") => Some(24454),
        None => None,
    };
    if let Some(port) = voice.filter(|p| *p != game_port) {
        ports.push(ExtraPort { udp: true, port });
    }
    ports
}

/// Advertise the tunnel to Minecraft's LAN discovery every 1.5 s, like a real LAN world.
async fn announce_on_lan(name: &str, port: u16) {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return;
    };
    let _ = socket.set_multicast_loop_v4(true);
    // Minecraft can't show '[' or ']' inside the MOTD tags.
    let motd = name.replace(['[', ']'], "");
    let packet = format!("[MOTD]{motd}[/MOTD][AD]{port}[/AD]");
    loop {
        let _ = socket.send_to(packet.as_bytes(), LAN_GROUP).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

/// Find a world this computer has opened to LAN, by listening for Minecraft's own LAN
/// announcements (the same ones the Multiplayer screen uses). Works for every version and
/// game language, unlike reading the port from the game log. `None` if nothing is announced
/// within `wait`.
pub async fn find_local_lan_world(wait: Duration) -> Option<u16> {
    let socket = lan_listener().ok()?;
    let mut buf = [0u8; 1024];
    let search = async {
        loop {
            let Ok((n, _)) = socket.recv_from(&mut buf).await else {
                continue;
            };
            let packet = String::from_utf8_lossy(&buf[..n]);
            // Skip worlds netherNode itself announces for friends' worlds.
            if packet.contains("(netherNode)") {
                continue;
            }
            let Some(port) = lan_ad_port(&packet) else {
                continue;
            };
            // Only a world on this computer: it must accept connections on localhost.
            let local = tokio::time::timeout(
                Duration::from_secs(1),
                TcpStream::connect((Ipv4Addr::LOCALHOST, port)),
            )
            .await;
            if matches!(local, Ok(Ok(_))) {
                return port;
            }
        }
    };
    tokio::time::timeout(wait, search).await.ok()
}

/// The port in a LAN announcement: `[MOTD]name[/MOTD][AD]port[/AD]`.
fn lan_ad_port(packet: &str) -> Option<u16> {
    let start = packet.find("[AD]")? + 4;
    let end = start + packet[start..].find("[/AD]")?;
    // Some versions announce "host:port".
    packet[start..end].rsplit(':').next()?.trim().parse().ok()
}

/// A UDP socket in Minecraft's LAN group, sharing the port with any game listening too.
fn lan_listener() -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), LAN_GROUP.port()).into())?;
    socket.join_multicast_v4(&Ipv4Addr::new(224, 0, 2, 60), &Ipv4Addr::UNSPECIFIED)?;
    socket.set_multicast_loop_v4(true)?;
    socket.set_nonblocking(true)?;
    UdpSocket::from_std(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_voice_chat_ports() {
        let dir = std::env::temp_dir().join(format!("nethernode-ports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        assert!(detect_mod_ports(&dir, 25565).is_empty());
        std::fs::write(dir.join("mods/voicechat-fabric-1.21.1-2.5.0.jar"), b"").unwrap();
        assert_eq!(
            detect_mod_ports(&dir, 25565),
            [ExtraPort {
                udp: true,
                port: 24454
            }]
        );
        std::fs::create_dir_all(dir.join("config/voicechat")).unwrap();
        std::fs::write(
            dir.join("config/voicechat/voicechat-server.properties"),
            "port=24500\n",
        )
        .unwrap();
        assert_eq!(
            detect_mod_ports(&dir, 25565),
            [ExtraPort {
                udp: true,
                port: 24500
            }]
        );
        std::fs::write(
            dir.join("config/voicechat/voicechat-server.properties"),
            "port=-1\n",
        )
        .unwrap();
        assert!(detect_mod_ports(&dir, 25565).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_lan_announcements() {
        assert_eq!(
            lan_ad_port("[MOTD]Steve - World[/MOTD][AD]51234[/AD]"),
            Some(51234)
        );
        assert_eq!(
            lan_ad_port("[MOTD]x[/MOTD][AD]192.168.1.5:40000[/AD]"),
            Some(40000)
        );
        assert_eq!(lan_ad_port("[MOTD]x[/MOTD]"), None);
    }

    /// Announces a fake world the way Minecraft does and finds it. Needs multicast on the
    /// loopback path, which some CI machines (GitHub's macOS runners) don't have.
    #[tokio::test]
    #[cfg_attr(target_os = "macos", ignore)]
    async fn finds_a_world_opened_to_lan() {
        let world = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = world.local_addr().unwrap().port();
        let announce = tokio::spawn(async move {
            let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.unwrap();
            let _ = socket.set_multicast_loop_v4(true);
            loop {
                let packet = format!("[MOTD]Steve - Test[/MOTD][AD]{port}[/AD]");
                let _ = socket.send_to(packet.as_bytes(), LAN_GROUP).await;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        });
        let found = find_local_lan_world(Duration::from_secs(5)).await;
        announce.abort();
        assert_eq!(found, Some(port));
    }

    #[test]
    fn join_codes_round_trip_and_reject_garbage() {
        let host = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let code = JoinCode {
            host,
            secret: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let text = code.to_string();
        assert_eq!(text.parse::<JoinCode>().unwrap(), code);
        assert_eq!(format!(" {text}\n").parse::<JoinCode>().unwrap(), code);
        for bad in [
            "",
            "hello",
            "abc-0102",
            &text[..text.len() - 2],
            &text.replace('-', ""),
        ] {
            assert!(bad.parse::<JoinCode>().is_err(), "{bad}");
        }
        assert!(!format!("{code:?}").contains("0102030405060708"));
    }
}
