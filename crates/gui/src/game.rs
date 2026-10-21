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
