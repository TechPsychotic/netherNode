//! Debug-build helper: `NETHERNODE_SNAPSHOT=<dir>` makes the app screenshot each screen to
//! `<dir>/<screen>.ppm` and exit. Used to check the UI without clicking through it by hand.
//!
//! - `NETHERNODE_SNAPSHOT_SCREENS=home,mods,...` picks the screens (default: all but play).
//! - `NETHERNODE_SNAPSHOT_INSTANCE=<name>` selects the instance first.
//! - `NETHERNODE_SNAPSHOT_CLOSE=1` instead starts the game and closes the window mid-game.
//!
//! Screens: home, new, new-forge, new-optifine, edit, accounts, mods, browse, playing
//! (presses Play, waits 25 s) and stopped (presses Stop).

use std::time::Duration;

use iced::{Task, window};
use nethernode_core::loader::LoaderKind;

use crate::{App, Message, ModsTab};

const DEFAULT: &str = "home,new,new-forge,new-optifine,edit,accounts,mods,browse";

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

fn screens() -> Vec<String> {
    std::env::var("NETHERNODE_SNAPSHOT_SCREENS")
        .unwrap_or_else(|_| DEFAULT.into())
        .split(',')
        .map(|s| s.trim().to_string())
        .collect()
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
    // Give the version list and textures time to load first.
    after(3500, Step::Show(0))
}

pub fn update(app: &mut App, step: Step) -> Task<Message> {
    let screens = screens();
    match step {
        Step::Close if app.runs.is_empty() => {
            let play = app.update(Message::Play(app.selected.clone().unwrap_or_default()));
            Task::batch([play, after(15_000, Step::Close)])
        }
        Step::Close => window::latest().and_then(|id| Task::done(Message::CloseRequested(id))),
        Step::Show(i) if i >= screens.len() => iced::exit(),
        Step::Show(i) => {
            if let Ok(name) = std::env::var("NETHERNODE_SNAPSHOT_INSTANCE") {
                app.selected = Some(name);
            }
            let selected = app.selected.clone().unwrap_or_default();
            let mut tasks = Vec::new();
            let mut send = |app: &mut App, message| tasks.push(app.update(message));
            let wait = match screens[i].as_str() {
                "new" => {
                    send(app, Message::ShowNewInstance);
                    send(app, Message::PickLoader(Some(LoaderKind::Fabric)));
                    2500
                }
                "new-forge" => {
                    send(app, Message::ShowNewInstance);
                    send(app, Message::NewVersion("1.20.1".into()));
                    send(app, Message::PickLoader(Some(LoaderKind::Forge)));
                    4000
                }
                "new-optifine" => {
                    send(app, Message::ShowNewInstance);
                    send(app, Message::PickLoader(Some(LoaderKind::OptiFine)));
                    800
                }
                "edit" => {
                    send(app, Message::ShowEdit(selected));
                    2500
                }
                "accounts" => {
                    send(app, Message::ShowAccounts);
                    800
                }
                "mods" => {
                    send(app, Message::ShowMods(selected));
                    1500
                }
                "browse" => {
                    send(app, Message::ShowMods(selected));
                    send(app, Message::ModsTab(ModsTab::Browse));
                    3500
                }
                "playing" => {
                    send(app, Message::ShowHome);
                    send(app, Message::Play(selected));
                    25_000
                }
                "stopped" => {
                    send(app, Message::Stop(selected));
                    4000
                }
                _ => {
                    send(app, Message::ShowHome);
                    800
                }
            };
            tasks.push(after(wait, Step::Capture(i)));
            Task::batch(tasks)
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
                let _ = std::fs::write(dir.join(format!("{}.ppm", screens[i])), ppm);
            }
            Task::done(Message::Snapshot(Step::Show(i + 1)))
        }
    }
}
