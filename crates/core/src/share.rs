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
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointId};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use crate::error::{Result, other};
use crate::skins::{self, SkinRecord, SkinStore};

pub const ALPN: &[u8] = b"nethernode/minecraft-lan/2";
/// First byte of every stream after approval: what the stream carries.
const STREAM_MINECRAFT: u8 = 1;
const STREAM_SKINS: u8 = 2;
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

async fn bind() -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0).alpns(vec![ALPN.to_vec()]);
    // Diagnostics: force the relay path, as on networks where hole punching fails.
    if std::env::var_os("NETHERNODE_SHARE_RELAY_ONLY").is_some() {
        builder = builder.clear_ip_transports();
    }
    builder
        .bind()
        .await
        .map_err(|e| other(format!("couldn't start networking: {e}")))
}

async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> std::io::Result<()> {
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    Ok(send.write_all(bytes).await?)
}

async fn read_frame(recv: &mut RecvStream) -> Option<Vec<u8>> {
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

/// Something the host's UI should show.
pub enum HostEvent {
    /// Sharing is live; give friends the short code (or the full one if it couldn't be
    /// published).
    Ready {
        code: JoinCode,
        short: Option<crate::invite::ShortCode>,
    },
    /// Someone with the right code wants in. Answer through `reply` (dropping it = deny).
    Request {
        friend: String,
        reply: oneshot::Sender<bool>,
    },
    Joined {
        friend: String,
    },
    Left {
        friend: String,
    },
}

/// Share `127.0.0.1:lan_port` (the world opened to LAN) until `stop` completes.
/// With `skins`, players' skins are exchanged too (see [`crate::skins::for_sharing`]).
pub async fn host(
    lan_port: u16,
    skins: Option<SkinStore>,
    on_event: impl Fn(HostEvent) + Send + Sync + 'static,
    stop: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let endpoint = bind().await?;
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

    let on_event = Arc::new(on_event);
    let approved: Arc<Mutex<HashSet<EndpointId>>> = Arc::default();
    let hub = skins.map(|store| Arc::new(SkinHub::new(store)));
    tokio::pin!(stop, republish);
    loop {
        tokio::select! {
            () = &mut republish => {}
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let (on_event, approved, hub) = (on_event.clone(), approved.clone(), hub.clone());
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else { return };
                    serve_friend(conn, secret, lan_port, on_event, approved, hub).await;
                });
            }
            () = &mut stop => break,
        }
    }
    endpoint.close().await;
    Ok(())
}

async fn serve_friend(
    conn: Connection,
    secret: [u8; SECRET_LEN],
    lan_port: u16,
    on_event: Arc<impl Fn(HostEvent) + Send + Sync + 'static>,
    approved: Arc<Mutex<HashSet<EndpointId>>>,
    hub: Option<Arc<SkinHub>>,
) {
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
    if !matches!(read, Ok(Ok(_))) || presented != secret {
        conn.close(1u32.into(), b"wrong code");
        return;
    }

    // 2. The host decides (once per friend per session).
    let known = approved.lock().unwrap().contains(&friend_id);
    if !known {
        let (reply, answer) = oneshot::channel();
        on_event(HostEvent::Request {
            friend: friend.clone(),
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
        approved.lock().unwrap().insert(friend_id);
    }
    let _ = control_send.write_all(&[1]).await;
    let _ = control_send.finish();
    on_event(HostEvent::Joined {
        friend: friend.clone(),
    });

    // 3. Every further stream is a Minecraft connection to the LAN world, or skins.
    while let Ok((send, mut recv)) = conn.accept_bi().await {
        let hub = hub.clone();
        tokio::spawn(async move {
            let mut kind = [0u8; 1];
            if recv.read_exact(&mut kind).await.is_err() {
                return;
            }
            match kind[0] {
                STREAM_MINECRAFT => {
                    if let Ok(tcp) = TcpStream::connect((Ipv4Addr::LOCALHOST, lan_port)).await {
                        pipe(tcp, send, recv).await;
                    }
                }
                STREAM_SKINS => {
                    if let Some(hub) = hub {
                        hub.serve(send, recv).await;
                    }
                }
                _ => {}
            }
        });
    }
    on_event(HostEvent::Left { friend });
}

/// Something the joiner's UI should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinEvent {
    Connecting,
    /// Connected; waiting for the host to click Allow.
    WaitingForApproval,
    /// In: Minecraft can connect to `localhost:port` (it's also in the LAN list).
    Ready {
        port: u16,
    },
}

/// Join a friend's world until `stop` completes or the host ends it.
/// With `skins`, our skins go to the host and everyone's come back.
pub async fn join(
    code: &JoinCode,
    name: &str,
    skins: Option<SkinStore>,
    on_event: impl Fn(JoinEvent) + Send + Sync + 'static,
    stop: impl std::future::Future<Output = ()>,
) -> Result<()> {
    on_event(JoinEvent::Connecting);
    let endpoint = bind().await?;
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

    // Listen on all interfaces so the LAN-list entry (which uses this computer's LAN address)
    // works, but only accept connections coming from this same computer.
    let skin_sync = match skins {
        Some(store) => tokio::spawn(exchange_skins(conn.clone(), store)),
        None => tokio::spawn(async {}),
    };
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let port = listener.local_addr()?.port();
    on_event(JoinEvent::Ready { port });

    let announce = announce_on_lan(name, port);
    tokio::pin!(stop, announce);
    let result = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((tcp, peer)) = accepted else { continue };
                let local = tcp.local_addr().map(|a| a.ip()).ok();
                if !(peer.ip().is_loopback() || Some(peer.ip()) == local) {
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
    conn.close(0u32.into(), b"bye");
    endpoint.close().await;
    result
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

    /// Announces a fake world the way Minecraft does and finds it.
    #[tokio::test]
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
