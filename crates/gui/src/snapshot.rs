//! Debug-build helper: `NETHERNODE_SNAPSHOT=<dir>` makes the app screenshot each screen to
//! `<dir>/<screen>.ppm` and exit. Used to check the UI without clicking through it by hand.

use std::time::Duration;

use iced::{Task, window};

use crate::{App, Message};

/// `playing` presses Play on the selected instance and `stopped` presses Stop. With
/// `NETHERNODE_SNAPSHOT_CLOSE` set, it instead starts the game and closes the window mid-game.
const SCREENS: &[&str] = &["home", "new", "edit", "accounts", "playing", "stopped"];

#[derive(Debug, Clone)]
pub enum Step {
    Show(usize),
    Capture(usize),
    Save(usize, window::Screenshot),
    Close,
}

fn dir() -> Option<std::path::PathBuf> {
    std::env::var_os("NETHERNODE_SNAPSHOT").map(Into::into)
}

fn after(ms: u64, step: Step) -> Task<Message> {
    Task::perform(tokio::time::sleep(Duration::from_millis(ms)), move |()| {
        Message::Snapshot(step.clone())
    })
}

pub fn start() -> Task<Message> {
    if dir().is_none() {
        return Task::none();
    }
    if std::env::var_os("NETHERNODE_SNAPSHOT_CLOSE").is_some() {
        return after(1000, Step::Close);
    }
    // Give the version list time to load first.
    after(3000, Step::Show(0))
}

pub fn update(app: &mut App, step: Step) -> Task<Message> {
    match step {
        Step::Close if app.runs.is_empty() => {
            let play = app.update(Message::Play(app.selected.clone().unwrap_or_default()));
            Task::batch([play, after(15_000, Step::Close)])
        }
        Step::Close => window::latest().and_then(|id| Task::done(Message::CloseRequested(id))),
        Step::Show(i) if i >= SCREENS.len() => iced::exit(),
        Step::Show(i) => {
            let selected = app.selected.clone().unwrap_or_default();
            let (message, wait) = match SCREENS[i] {
                "new" => (Message::ShowNewInstance, 600),
                "edit" => (Message::ShowEdit(selected), 600),
                "accounts" => (Message::ShowAccounts, 600),
                "playing" => {
                    let _ = app.update(Message::ShowHome);
                    (Message::Play(selected), 25_000)
                }
                "stopped" => (Message::Stop(selected), 4000),
                _ => (Message::ShowHome, 600),
            };
            let task = app.update(message);
            Task::batch([task, after(wait, Step::Capture(i))])
        }
        Step::Capture(i) => window::latest()
            .and_then(window::screenshot)
            .map(move |shot| Message::Snapshot(Step::Save(i, shot))),
        Step::Save(i, shot) => {
            if let Some(dir) = dir() {
                let mut ppm =
                    format!("P6\n{} {}\n255\n", shot.size.width, shot.size.height).into_bytes();
                ppm.extend(
                    shot.rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|px| [px[0], px[1], px[2]]),
                );
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join(format!("{}.ppm", SCREENS[i])), ppm);
            }
            Task::done(Message::Snapshot(Step::Show(i + 1)))
        }
    }
}
