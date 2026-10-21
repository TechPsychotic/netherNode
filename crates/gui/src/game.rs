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
            cmd = launch::prepare(&paths, &instance, &account, None, &progress).fuse() => Some(cmd),
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
    Ready(String),
    /// A friend with the right code is asking to join; answer through the sender.
    Request(String, JoinReply),
    Joined(String),
    Left(String),
    Ended(Result<(), String>),
}

/// Share a world opened to LAN on `port` until `stop` fires.
pub fn host_share(port: u16, stop: oneshot::Receiver<()>) -> impl Stream<Item = ShareEvent> {
    use nethernode_core::share::{self, HostEvent};
    let (tx, rx) = mpsc::unbounded();
    let events = tx.clone();
    let run = async move {
        let result = share::host(
            port,
            move |event| {
                let event = match event {
                    HostEvent::Ready(code) => ShareEvent::Ready(code.to_string()),
                    HostEvent::Request { friend, reply } => {
                        ShareEvent::Request(friend, Arc::new(std::sync::Mutex::new(Some(reply))))
                    }
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

/// Join a friend's world until `stop` fires or the host ends it.
pub fn join_share(
    code: nethernode_core::share::JoinCode,
    stop: oneshot::Receiver<()>,
) -> impl Stream<Item = JoinUpdate> {
    let (tx, rx) = mpsc::unbounded();
    let events = tx.clone();
    let run = async move {
        let result = nethernode_core::share::join(
            &code,
            "Friend's world (netherNode)",
            move |event| {
                let _ = events.unbounded_send(JoinUpdate::Status(event));
            },
            stop.map(|_| ()),
        )
        .await;
        let _ = tx.unbounded_send(JoinUpdate::Ended(result.map_err(|e| e.to_string())));
    };
    stream::select(rx, stream::once(run).filter_map(|()| async { None }))
}
