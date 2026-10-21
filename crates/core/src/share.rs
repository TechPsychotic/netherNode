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
//! Each Minecraft TCP connection (server-list ping, game) is one QUIC stream.

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

pub const ALPN: &[u8] = b"nethernode/minecraft-lan/1";
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

/// Something the host's UI should show.
pub enum HostEvent {
    /// Sharing is live; give this code to friends.
    Ready(JoinCode),
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
pub async fn host(
    lan_port: u16,
    on_event: impl Fn(HostEvent) + Send + Sync + 'static,
    stop: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let endpoint = bind().await?;
    // Wait until reachable via a relay, so the code works straight away.
    let _ = tokio::time::timeout(Duration::from_secs(20), endpoint.online()).await;
    let mut secret = [0u8; SECRET_LEN];
    getrandom::fill(&mut secret).map_err(|e| other(format!("no secure random numbers: {e}")))?;
    on_event(HostEvent::Ready(JoinCode {
        host: endpoint.id(),
        secret,
    }));

    let on_event = Arc::new(on_event);
    let approved: Arc<Mutex<HashSet<EndpointId>>> = Arc::default();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let (on_event, approved) = (on_event.clone(), approved.clone());
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else { return };
                    serve_friend(conn, secret, lan_port, on_event, approved).await;
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

    // 3. Every further stream is one Minecraft connection to the LAN world.
    while let Ok((send, recv)) = conn.accept_bi().await {
        tokio::spawn(async move {
            if let Ok(tcp) = TcpStream::connect((Ipv4Addr::LOCALHOST, lan_port)).await {
                pipe(tcp, send, recv).await;
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
pub async fn join(
    code: &JoinCode,
    name: &str,
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
                    if let Ok((send, recv)) = conn.open_bi().await {
                        pipe(tcp, send, recv).await;
                    }
                });
            }
            reason = conn.closed() => break Err(other(format!("the host ended the session ({reason})"))),
            () = &mut announce => {}
            () = &mut stop => break Ok(()),
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;

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
