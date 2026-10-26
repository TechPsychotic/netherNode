//! Bridges a core launch (download, then run) into a stream of events for the UI.

use std::sync::Arc;

use futures::channel::{mpsc, oneshot};
use futures::future::{FutureExt, Shared};
use futures::{Stream, StreamExt, stream};
use nethernode_core::account::Account;
use nethernode_core::crash::CrashInfo;
use nethernode_core::download::{Progress, ProgressFn};
use nethernode_core::instance::Instance;
use nethernode_core::launch::{self, RunOutcome};
use nethernode_core::paths::Paths;

#[derive(Debug, Clone)]
pub enum Event {
    /// A download stage started.
    Stage {
        name: String,
        total: u64,
    },
    /// One item of the current stage finished.
    Advance,
    /// The game process started with this heap.
    Started {
        heap: String,
        warning: Option<String>,
    },
    Log(String),
    /// The run is over.
    Finished(Ended),
}

#[derive(Debug, Clone)]
pub enum Ended {
    /// Normal end, with a short status line.
    Ok(String),
    /// Couldn't get as far as starting the game (download error, bad Java...).
    Failed(String),
    /// The game started but crashed or exited with an error.
    Crashed(CrashInfo),
}

/// Download what is missing, start the game and report everything as [`Event`]s.
///
/// Sending on (or dropping) the `stop` sender cancels downloads or kills the game.
/// Dropping the stream itself also kills the game, since the child is `kill_on_drop`.
pub fn launch(
    paths: Paths,
    instance: Instance,
    account: Account,
    quick_play: Option<launch::QuickPlay>,
    stop: oneshot::Receiver<()>,
) -> impl Stream<Item = Event> {
    let (tx, rx) = mpsc::unbounded();
    let stop: Shared<_> = stop.map(|_| ()).shared();

    let work = async move {
        let progress_tx = tx.clone();
        let progress: ProgressFn = Arc::new(move |event| {
            let event = match event {
                Progress::Stage { name, total } => Event::Stage { name, total },
                Progress::Advance => Event::Advance,
            };
            let _ = progress_tx.unbounded_send(event);
        });

        let prepared = futures::select! {
            cmd = launch::prepare(&paths, &instance, &account, quick_play, &progress).fuse() => Some(cmd),
            () = stop.clone() => None,
        };
        let ended = match prepared {
            None => Ended::Ok("Cancelled".into()),
            Some(Err(e)) => Ended::Failed(e.to_string()),
            Some(Ok(cmd)) => {
                let heap = format!("{}–{} MB", cmd.heap.min_mb, cmd.heap.max_mb)
                    + if cmd.heap.low_memory {
                        ", low-memory mode"
                    } else {
                        ""
                    };
                let _ = tx.unbounded_send(Event::Started {
                    heap,
                    warning: cmd.heap.warning.clone(),
                });
                let log_tx = tx.clone();
                let on_line = move |line: nethernode_core::log::LogLine| {
                    let _ = log_tx.unbounded_send(Event::Log(line.to_string()));
                };
                match launch::run(&cmd, on_line, stop).await {
                    Ok(RunOutcome::Exited {
                        crash: Some(crash), ..
                    }) => Ended::Crashed(crash),
                    Ok(RunOutcome::Exited { crash: None, .. }) => Ended::Ok("Game closed".into()),
                    Ok(RunOutcome::Stopped) => Ended::Ok("Game stopped".into()),
                    Err(e) => Ended::Failed(e.to_string()),
                }
            }
        };
        let _ = tx.unbounded_send(Event::Finished(ended));
    };

    stream::select(rx, stream::once(work).filter_map(|()| async { None }))
}

/// Progress of a background job such as installing a mod loader.
#[derive(Debug, Clone)]
pub enum JobEvent {
    Stage {
        name: String,
        total: u64,
    },
    Advance,
    /// Finished: a short success message, or the error.
    Done(Result<String, String>),
}

/// Run `work` (given a progress callback) and report it as [`JobEvent`]s.
pub fn job<F, Fut>(work: F) -> impl Stream<Item = JobEvent>
where
    F: FnOnce(ProgressFn) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded();
    let progress_tx = tx.clone();
    let progress: ProgressFn = Arc::new(move |event| {
        let event = match event {
            Progress::Stage { name, total } => JobEvent::Stage { name, total },
            Progress::Advance => JobEvent::Advance,
        };
        let _ = progress_tx.unbounded_send(event);
    });
    let run = async move {
        let result = work(progress).await;
        let _ = tx.unbounded_send(JobEvent::Done(result));
    };
    stream::select(rx, stream::once(run).filter_map(|()| async { None }))
}

/// The host's answer to a join request; shared so the event can be cloned.
pub type JoinReply = Arc<std::sync::Mutex<Option<oneshot::Sender<bool>>>>;

/// Host-side sharing events, made cloneable for iced messages.
#[derive(Debug, Clone)]
pub enum ShareEvent {
    /// The code to show (four words when possible) and the full code.
    Ready(String, String),
    /// Someone with the right code is asking to join: their short name, their netherNode id
    /// (to let friends in automatically) and where to answer.
    Request(String, String, JoinReply),
    Joined(String),
    Left(String),
    Ended(Result<(), String>),
}

/// Share a Minecraft port until `stop` fires. `options` is prepared first (it may look up
/// mods on Modrinth).
pub fn host_share(
    options: impl std::future::Future<Output = nethernode_core::share::HostOptions> + Send + 'static,
    stop: oneshot::Receiver<()>,
) -> impl Stream<Item = ShareEvent> {
    use nethernode_core::share::{self, HostEvent};
    let (tx, rx) = mpsc::unbounded();
    let events = tx.clone();
    let run = async move {
        let options = options.await;
        let result = share::host(
            options,
            move |event| {
                let event = match event {
                    HostEvent::Ready { code, short } => ShareEvent::Ready(
                        short.map_or_else(|| code.to_string(), |s| s.to_string()),
                        code.to_string(),
                    ),
                    HostEvent::Request { friend, id, reply } => ShareEvent::Request(
                        friend,
                        id.to_z32(),
                        Arc::new(std::sync::Mutex::new(Some(reply))),
                    ),
                    HostEvent::Joined { friend } => ShareEvent::Joined(friend),
                    HostEvent::Left { friend } => ShareEvent::Left(friend),
                };
                let _ = events.unbounded_send(event);
            },
            stop.map(|_| ()),
        )
        .await;
        let _ = tx.unbounded_send(ShareEvent::Ended(result.map_err(|e| e.to_string())));
    };
    stream::select(rx, stream::once(run).filter_map(|()| async { None }))
}

/// Joiner-side events.
#[derive(Debug, Clone)]
pub enum JoinUpdate {
    Status(nethernode_core::share::JoinEvent),
    Ended(Result<(), String>),
}

/// Join a friend's world until `stop` fires or the host ends it. `via` is the friends
/// endpoint, so a friend's netherNode lets us in without asking.
pub fn join_share(
    paths: Paths,
    code: String,
    via: Option<nethernode_core::friends::Endpoint>,
    stop: oneshot::Receiver<()>,
) -> impl Stream<Item = JoinUpdate> {
    let (tx, rx) = mpsc::unbounded();
    let events = tx.clone();
    let run = async move {
        let result = async {
            let code = nethernode_core::invite::resolve(&code).await?;
            nethernode_core::share::join(
                &code,
                "Friend's world (netherNode)",
                nethernode_core::skins::for_sharing(&paths),
                via,
                move |event| {
                    let _ = events.unbounded_send(JoinUpdate::Status(event));
                },
                stop.map(|_| ()),
            )
            .await
        }
        .await;
        let _ = tx.unbounded_send(JoinUpdate::Ended(result.map_err(|e| e.to_string())));
    };
    stream::select(rx, stream::once(run).filter_map(|()| async { None }))
}

/// Make an instance with a friend's mods, reporting progress like other jobs.
pub fn get_friend_mods(
    paths: Paths,
    manifest: nethernode_core::modsync::Manifest,
    source: nethernode_core::share::ModSource,
    name: String,
) -> impl Stream<Item = JobEvent> {
    job(move |progress| async move {
        let http = nethernode_core::download::client();
        let fetch = |sha1: String| {
            let source = source.clone();
            async move { source.fetch(sha1).await }
        };
        nethernode_core::modsync::create_instance(&http, &paths, &manifest, &name, fetch, &progress)
            .await
            .map(|i| format!("{} is ready with your friend's mods", i.name))
            .map_err(|e| e.to_string())
    })
}

/// Progress and output of a server hosted on this PC.
#[derive(Debug, Clone)]
pub enum ServerUpdate {
    Stage { name: String, total: u64 },
    Advance,
    Started { heap_mb: u64 },
    Line(String),
    Ended(Result<String, String>),
}

/// Start `server` (downloading Java if needed) and stream its console until it exits.
/// Lines sent on `commands` go to its console; `stop` saves and stops it.
pub fn run_server(
    paths: Paths,
    server: nethernode_core::server::Server,
    commands: mpsc::UnboundedReceiver<String>,
    stop: oneshot::Receiver<()>,
) -> impl Stream<Item = ServerUpdate> {
    use nethernode_core::server;
    let (tx, rx) = mpsc::unbounded();
    let run = async move {
        let progress_tx = tx.clone();
        let progress: ProgressFn = Arc::new(move |event| {
            let event = match event {
                Progress::Stage { name, total } => ServerUpdate::Stage { name, total },
                Progress::Advance => ServerUpdate::Advance,
            };
            let _ = progress_tx.unbounded_send(event);
        });
        let result = async {
            let cmd = server::command(
                &nethernode_core::download::client(),
                &paths,
                &server,
                &progress,
            )
            .await
            .map_err(|e| e.to_string())?;
            let _ = tx.unbounded_send(ServerUpdate::Started {
                heap_mb: cmd.heap_mb,
            });
            let lines = tx.clone();
            let status = server::run(
                &cmd,
                move |line| {
                    let _ = lines.unbounded_send(ServerUpdate::Line(line));
                },
                commands,
                stop.map(|_| ()),
            )
            .await
            .map_err(|e| e.to_string())?;
            Ok(if status.success() {
                "Server stopped".to_string()
            } else {
                format!("Server exited ({status})")
            })
        }
        .await;
        let _ = tx.unbounded_send(ServerUpdate::Ended(result));
    };
    stream::select(rx, stream::once(run).filter_map(|()| async { None }))
}

/// The permanent-friends service's events.
#[derive(Debug, Clone)]
pub enum FriendsUpdate {
    Started(nethernode_core::friends::Friends),
    Failed(String),
    /// The list or someone's presence changed.
    Changed,
}

/// Start the friends service and report changes for as long as the app runs.
pub fn friends_service(paths: Paths, name: String) -> impl Stream<Item = FriendsUpdate> {
    let (tx, rx) = mpsc::unbounded();
    let changes = tx.clone();
    let run = async move {
        let started = nethernode_core::friends::Friends::start(&paths, &name, move || {
            let _ = changes.unbounded_send(FriendsUpdate::Changed);
        })
        .await;
        let _ = tx.unbounded_send(match started {
            Ok(friends) => FriendsUpdate::Started(friends),
            Err(e) => FriendsUpdate::Failed(e.to_string()),
        });
    };
    stream::select(rx, stream::once(run).filter_map(|()| async { None }))
}
