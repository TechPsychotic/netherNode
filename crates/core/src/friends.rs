//! Permanent friends. Each netherNode has a lasting identity (a key kept in the data folder).
//! Friends add each other once with a four-word friend code; after that their netherNodes
//! check on each other over iroh every so often, so each side sees whether the other is
//! online and what they're sharing (a world opened to LAN or a server), and joins it with
//! one click. Friends are let in without the Allow prompt.
//!
//! Only people on your friends list can talk to your netherNode, apart from a friend
//! request, which you accept or decline.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, presets};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

use crate::error::{Result, other};
use crate::invite::ShortCode;
use crate::paths::{Paths, ensure_parent};
use crate::share::{read_frame, write_frame};

/// The iroh endpoint type, for joining through [`Friends::endpoint`].
pub use iroh::Endpoint;

pub const ALPN: &[u8] = b"nethernode/friends/1";
const CODE_DOMAIN: &str = "nethernode/friend-code/1";
const CODE_RECORD: &str = "_nethernode-friend";
/// How often friends are checked on, and how long one counts as online after an answer.
const CHECK_EVERY: Duration = Duration::from_secs(20);
const ONLINE_FOR: Duration = Duration::from_secs(65);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Friend {
    /// Their netherNode's id.
    pub id: String,
    pub name: String,
    /// They accepted (or added) us too. Until then they're "waiting".
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    friends: Vec<Friend>,
    /// Friend requests waiting for an answer.
    #[serde(default)]
    requests: Vec<Friend>,
}

/// Something a friend is sharing right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shared {
    /// World or server name.
    pub title: String,
    /// The full join code.
    pub code: String,
    pub server: bool,
    /// e.g. "Minecraft 1.21.1 · Fabric 0.16.5 · 12 mods".
    pub summary: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Note {
    Hello { name: String, sharing: Vec<Shared> },
    Request { name: String },
    Accept { name: String },
    Remove,
}

/// What we last heard from a friend.
#[derive(Debug, Clone, Default)]
pub struct Presence {
    pub online: bool,
    pub sharing: Vec<Shared>,
}

/// The friends service: an iroh endpoint with this computer's identity, the friends list and
/// their presence. Cheap to clone.
#[derive(Clone)]
pub struct Friends {
    inner: Arc<Inner>,
}

struct Inner {
    endpoint: Endpoint,
    file: PathBuf,
    book: Mutex<Book>,
    seen: Mutex<HashMap<String, (Instant, Vec<Shared>)>>,
    me: Mutex<String>,
    sharing: Mutex<Vec<Shared>>,
    poke: tokio::sync::Notify,
    on_change: Box<dyn Fn() + Send + Sync>,
}

/// This computer's lasting identity, created on first use.
fn identity(paths: &Paths) -> Result<SecretKey> {
    let file = paths.root.join("identity.key");
    if let Ok(text) = std::fs::read_to_string(&file)
        && let Ok(bytes) = hex::decode(text.trim())
        && let Ok(bytes) = <[u8; 32]>::try_from(bytes)
    {
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| other(format!("no secure random numbers: {e}")))?;
    ensure_parent(&file)?;
    std::fs::write(&file, hex::encode(bytes))?;
    Ok(SecretKey::from_bytes(&bytes))
}

fn parse_id(id: &str) -> Result<EndpointId> {
    EndpointId::from_z32(id).map_err(|_| other("that isn't a netherNode friend id"))
}

impl Friends {
    /// Start the friends service. `on_change` is called whenever the list or someone's
    /// presence changes (from a background task).
    pub async fn start(
        paths: &Paths,
        my_name: &str,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self> {
        let mut builder = Endpoint::builder(presets::N0)
            .alpns(vec![ALPN.to_vec()])
            .secret_key(identity(paths)?);
        if std::env::var_os("NETHERNODE_SHARE_RELAY_ONLY").is_some() {
            builder = builder.clear_ip_transports();
        }
        let endpoint = builder
            .bind()
            .await
            .map_err(|e| other(format!("couldn't start networking: {e}")))?;
        // Until it's reachable through a relay, friends can't find it.
        let _ = tokio::time::timeout(Duration::from_secs(15), endpoint.online()).await;
        let file = paths.root.join("friends.json");
        let book = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let friends = Self {
            inner: Arc::new(Inner {
                endpoint,
                file,
                book: Mutex::new(book),
                seen: Mutex::default(),
                me: Mutex::new(my_name.to_string()),
                sharing: Mutex::default(),
                poke: tokio::sync::Notify::new(),
                on_change: Box::new(on_change),
            }),
        };
        tokio::spawn(friends.clone().accept_loop());
        tokio::spawn(friends.clone().check_loop());
        Ok(friends)
    }

    /// This computer's friend id.
    pub fn id(&self) -> String {
        self.inner.endpoint.id().to_z32()
    }

    /// The endpoint to join friends' worlds through, so they recognise us.
    pub fn endpoint(&self) -> Endpoint {
        self.inner.endpoint.clone()
    }

    /// Friends with what we last heard from them.
    pub fn list(&self) -> Vec<(Friend, Presence)> {
        let seen = self.inner.seen.lock().unwrap();
        self.inner
            .book
            .lock()
            .unwrap()
            .friends
            .iter()
            .map(|f| {
                let presence = match seen.get(&f.id) {
                    Some((at, sharing)) if at.elapsed() < ONLINE_FOR => Presence {
                        online: true,
                        sharing: sharing.clone(),
                    },
                    _ => Presence::default(),
                };
                (f.clone(), presence)
            })
            .collect()
    }

    /// Friend requests waiting for an answer.
    pub fn requests(&self) -> Vec<Friend> {
        self.inner.book.lock().unwrap().requests.clone()
    }

    /// Whether `id` is a confirmed friend (let in without asking).
    pub fn is_friend(&self, id: &EndpointId) -> bool {
        let id = id.to_z32();
        self.inner
            .book
            .lock()
            .unwrap()
            .friends
            .iter()
            .any(|f| f.id == id && f.confirmed)
    }

    /// A confirmed friend's name, by their netherNode id (as shown in join requests).
    pub fn friend_name(&self, id: &str) -> Option<String> {
        self.inner
            .book
            .lock()
            .unwrap()
            .friends
            .iter()
            .find(|f| f.id == id && f.confirmed)
            .map(|f| f.name.clone())
    }

    pub fn set_name(&self, name: &str) {
        *self.inner.me.lock().unwrap() = name.to_string();
    }

    /// What we're sharing now; friends hear about it within seconds.
    pub fn set_sharing(&self, sharing: Vec<Shared>) {
        *self.inner.sharing.lock().unwrap() = sharing;
        self.inner.poke.notify_one();
    }

    /// Publish a new four-word friend code for this computer (valid for about an hour).
    pub async fn friend_code(&self) -> Result<ShortCode> {
        let code = ShortCode::random()?;
        crate::invite::publish_record(
            &crate::download::client(),
            &code.key_for(CODE_DOMAIN),
            CODE_RECORD,
            &self.id(),
        )
        .await?;
        Ok(code)
    }

    /// Add a friend by their four-word friend code (or their friend id) and send them a
    /// request. They show as waiting until they accept.
    pub async fn add(&self, code: &str) -> Result<Friend> {
        let id = match parse_id(code.trim()) {
            Ok(id) => id,
            Err(_) => {
                let words = ShortCode::parse(code).ok_or_else(|| {
                    other("a friend code is four words, like maple-orbit-tunnel-frog")
                })?;
                let found = crate::invite::lookup_record(
                    &crate::download::client(),
                    &words.key_for(CODE_DOMAIN).public(),
                    CODE_RECORD,
                )
                .await?;
                found.iter().find_map(|v| parse_id(v).ok()).ok_or_else(|| {
                    other("no one has that friend code right now; ask your friend for a fresh one")
                })?
            }
        };
        if id == self.inner.endpoint.id() {
            return Err(other("that's your own friend code"));
        }
        let me = self.inner.me.lock().unwrap().clone();
        let reply = self.send(id, &Note::Request { name: me }).await?;
        let name = match reply {
            Some(Note::Hello { name, .. } | Note::Accept { name }) => name,
            _ => "Friend".to_string(),
        };
        let friend = Friend {
            id: id.to_z32(),
            name,
            confirmed: false,
        };
        {
            let mut book = self.inner.book.lock().unwrap();
            // They may have asked us first: then this is a yes.
            let asked_us = book.requests.iter().any(|r| r.id == friend.id);
            book.requests.retain(|r| r.id != friend.id);
            book.friends.retain(|f| f.id != friend.id);
            book.friends.push(Friend {
                confirmed: asked_us,
                ..friend.clone()
            });
        }
        self.save();
        self.inner.poke.notify_one();
        Ok(friend)
    }

    /// Accept a friend request.
    pub async fn accept(&self, id: &str) -> Result<()> {
        let request = {
            let mut book = self.inner.book.lock().unwrap();
            let Some(i) = book.requests.iter().position(|r| r.id == id) else {
                return Ok(());
            };
            let request = book.requests.remove(i);
            book.friends.retain(|f| f.id != id);
            book.friends.push(Friend {
                confirmed: true,
                ..request.clone()
            });
            request
        };
        self.save();
        let me = self.inner.me.lock().unwrap().clone();
        let _ = self
            .send(parse_id(&request.id)?, &Note::Accept { name: me })
            .await;
        self.inner.poke.notify_one();
        Ok(())
    }

    pub fn decline(&self, id: &str) {
        self.inner
            .book
            .lock()
            .unwrap()
            .requests
            .retain(|r| r.id != id);
        self.save();
    }

    /// Remove a friend (and tell them, if they're online).
    pub async fn remove(&self, id: &str) {
        self.inner
            .book
            .lock()
            .unwrap()
            .friends
            .retain(|f| f.id != id);
        self.inner.seen.lock().unwrap().remove(id);
        self.save();
        if let Ok(id) = parse_id(id) {
            let _ = self.send(id, &Note::Remove).await;
        }
    }

    pub async fn shutdown(&self) {
        self.inner.endpoint.close().await;
    }

    fn save(&self) {
        let book = self.inner.book.lock().unwrap().clone();
        if let Ok(bytes) = serde_json::to_vec_pretty(&book) {
            let _ = ensure_parent(&self.inner.file);
            let _ = std::fs::write(&self.inner.file, bytes);
        }
        (self.inner.on_change)();
    }

    fn hello(&self, to_confirmed_friend: bool) -> Note {
        Note::Hello {
            name: self.inner.me.lock().unwrap().clone(),
            // Join codes only go to confirmed friends.
            sharing: if to_confirmed_friend {
                self.inner.sharing.lock().unwrap().clone()
            } else {
                Vec::new()
            },
        }
    }

    async fn connect(&self, to: EndpointId) -> Result<Connection> {
        let attempt = || {
            tokio::time::timeout(
                Duration::from_secs(12),
                self.inner.endpoint.connect(to, ALPN),
            )
        };
        let first = attempt().await;
        let result = match first {
            Ok(Ok(conn)) => return Ok(conn),
            // A netherNode that just started may not be findable yet: try once more.
            _ => {
                tokio::time::sleep(Duration::from_secs(3)).await;
                attempt().await
            }
        };
        result
            .map_err(|_| other("your friend's netherNode isn't reachable right now"))?
            .map_err(|e| other(format!("couldn't reach your friend: {e}")))
    }

    /// Send one note and wait for the answer (if any).
    async fn send(&self, to: EndpointId, note: &Note) -> Result<Option<Note>> {
        let conn = self.connect(to).await?;
        let result = async {
            let (mut send, mut recv) = conn.open_bi().await.ok()?;
            write_frame(&mut send, &serde_json::to_vec(note).ok()?)
                .await
                .ok()?;
            let _ = send.finish();
            let reply = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut recv))
                .await
                .ok()??;
            serde_json::from_slice::<Note>(&reply).ok()
        }
        .await;
        conn.close(0u32.into(), b"bye");
        Ok(result)
    }

    async fn accept_loop(self) {
        while let Some(incoming) = self.inner.endpoint.accept().await {
            let this = self.clone();
            tokio::spawn(async move {
                if let Ok(conn) = incoming.await {
                    this.answer(conn).await;
                }
            });
        }
    }

    /// Answer one note from someone.
    async fn answer(&self, conn: Connection) {
        let from = conn.remote_id().to_z32();
        let Ok(Ok((mut send, mut recv))) =
            tokio::time::timeout(Duration::from_secs(10), conn.accept_bi()).await
        else {
            return;
        };
        let Some(note) = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut recv))
            .await
            .ok()
            .flatten()
            .and_then(|f| serde_json::from_slice::<Note>(&f).ok())
        else {
            return;
        };
        let known = self
            .inner
            .book
            .lock()
            .unwrap()
            .friends
            .iter()
            .find(|f| f.id == from)
            .cloned();
        let reply = match (note, known) {
            (Note::Request { name }, Some(_)) => {
                // We added them too: now it's mutual.
                self.confirm(&from, &name);
                Some(self.hello(true))
            }
            (Note::Request { name }, None) => {
                let name: String = name.chars().filter(|c| !c.is_control()).take(32).collect();
                {
                    let mut book = self.inner.book.lock().unwrap();
                    book.requests.retain(|r| r.id != from);
                    book.requests.push(Friend {
                        id: from.clone(),
                        name,
                        confirmed: false,
                    });
                }
                self.save();
                // Just our name, so their list shows who they asked.
                Some(self.hello(false))
            }
            (Note::Accept { name }, Some(_)) => {
                self.confirm(&from, &name);
                None
            }
            (Note::Hello { name, sharing }, Some(friend)) => {
                if !friend.confirmed {
                    // They know us, so they accepted.
                    self.confirm(&from, &name);
                }
                self.seen(&from, sharing);
                Some(self.hello(true))
            }
            (Note::Remove, Some(_)) => {
                self.inner
                    .book
                    .lock()
                    .unwrap()
                    .friends
                    .retain(|f| f.id != from);
                self.inner.seen.lock().unwrap().remove(&from);
                self.save();
                None
            }
            // Strangers get nothing but the friend-request path.
            _ => None,
        };
        if let Some(reply) = reply
            && let Ok(bytes) = serde_json::to_vec(&reply)
        {
            let _ = write_frame(&mut send, &bytes).await;
        }
        let _ = send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    }

    fn confirm(&self, id: &str, name: &str) {
        let name: String = name.chars().filter(|c| !c.is_control()).take(32).collect();
        {
            let mut book = self.inner.book.lock().unwrap();
            if let Some(f) = book.friends.iter_mut().find(|f| f.id == id) {
                f.confirmed = true;
                if !name.is_empty() {
                    f.name = name;
                }
            }
        }
        self.save();
    }

    fn seen(&self, id: &str, sharing: Vec<Shared>) {
        let changed = {
            let mut seen = self.inner.seen.lock().unwrap();
            let was = seen
                .get(id)
                .filter(|(at, _)| at.elapsed() < ONLINE_FOR)
                .map(|(_, s)| s.clone());
            seen.insert(id.to_string(), (Instant::now(), sharing.clone()));
            was.as_ref() != Some(&sharing)
        };
        if changed {
            (self.inner.on_change)();
        }
    }

    /// Say hello to every friend now and then (and right away when what we share changes).
    async fn check_loop(self) {
        loop {
            let friends = self.inner.book.lock().unwrap().friends.clone();
            let checks = friends.into_iter().map(|f| {
                let this = self.clone();
                async move {
                    let Ok(id) = parse_id(&f.id) else { return };
                    let hello = this.hello(f.confirmed);
                    match this.send(id, &hello).await {
                        Ok(Some(Note::Hello { name, sharing })) => {
                            if !f.confirmed || f.name != name {
                                this.confirm(&f.id, &name);
                            }
                            this.seen(&f.id, sharing);
                        }
                        _ => {
                            // Unreachable: they'll show offline once ONLINE_FOR passes.
                            let was_online = this
                                .inner
                                .seen
                                .lock()
                                .unwrap()
                                .get(&f.id)
                                .is_some_and(|(at, _)| at.elapsed() < ONLINE_FOR);
                            if !was_online {
                                (this.inner.on_change)();
                            }
                        }
                    }
                }
            });
            futures::future::join_all(checks).await;
            // Presence also changes by time passing (someone went quiet).
            (self.inner.on_change)();
            tokio::select! {
                () = tokio::time::sleep(CHECK_EVERY) => {}
                () = self.inner.poke.notified() => {}
            }
        }
    }
}

impl std::fmt::Debug for Friends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Friends({})", self.inner.endpoint.id().fmt_short())
    }
}
