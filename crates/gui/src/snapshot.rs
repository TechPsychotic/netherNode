//! Debug-build helper: `NETHERNODE_SNAPSHOT=<dir>` makes the app screenshot each screen to
//! `<dir>/<screen>.ppm` and exit. Used to check the UI without clicking through it by hand.
//!
//! - `NETHERNODE_SNAPSHOT_SCREENS=home,mods,...` picks the screens (default: all but play).
//! - `NETHERNODE_SNAPSHOT_INSTANCE=<name>` selects the instance first.
//! - `NETHERNODE_SNAPSHOT_CLOSE=1` instead starts the game and closes the window mid-game.
//!
//! Screens: home, settings, new-snapshot, new, new-all, new-forge, new-optifine, edit, accounts, mods, browse,
//! modpacks, playing (presses Play, waits 25 s), stopped (presses Stop), drop-pack (drops
//! `$NETHERNODE_SNAPSHOT_PACK` on the window, captures after 5 s) and home-later (150 s wait).

use std::time::Duration;

use iced::{Task, window};
use nethernode_core::loader::LoaderKind;

use crate::{App, Message, ModsTab, VersionFilter};

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
                "host-share" => {
                    // Pretend the selected instance is running with its world open to LAN
                    // on $NETHERNODE_SNAPSHOT_LAN_PORT, then invite friends.
                    let port = std::env::var("NETHERNODE_SNAPSHOT_LAN_PORT")
                        .ok()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(5555);
                    app.runs.insert(
                        selected.clone(),
                        crate::Run {
                            stage: None,
                            heap: Some("test".into()),
                            stop: None,
                            lan_port: Some(port),
                        },
                    );
                    send(app, Message::ShowHome);
                    send(app, Message::InviteFriends(selected));
                    20_000
                }
                "host-request" => {
                    // Publish the code for an outside joiner, then wait for its request.
                    if let (Some(dir), Some(code)) =
                        (dir(), app.shares.first().and_then(|s| s.code.clone()))
                    {
                        let _ = std::fs::write(dir.join("code.txt"), code);
                    }
                    25_000
                }
                "host-allow" => {
                    if let Some(key) = app
                        .shares
                        .first()
                        .map(|s| crate::share_key(s.is_server, &s.instance))
                    {
                        send(app, Message::AnswerJoin(key, 0, true));
                    }
                    6000
                }
                "join-friend" => {
                    let code = std::env::var("NETHERNODE_SNAPSHOT_CODE").unwrap_or_default();
                    send(app, Message::ShowJoinFriend);
                    send(app, Message::JoinCodeInput(code));
                    send(app, Message::StartJoin);
                    12_000
                }
                "servers" => {
                    send(app, Message::ShowServers);
                    if let Ok(server) = std::env::var("NETHERNODE_SNAPSHOT_SERVER") {
                        send(app, Message::SelectServer(server));
                    }
                    1000
                }
                "server-start" => {
                    if let Ok(server) = std::env::var("NETHERNODE_SNAPSHOT_SERVER") {
                        send(app, Message::StartServer(server));
                    }
                    90_000
                }
                "server-invite" => {
                    if let Ok(server) = std::env::var("NETHERNODE_SNAPSHOT_SERVER") {
                        send(app, Message::InviteServer(server));
                    }
                    15_000
                }
                "server-stop" => {
                    if let Ok(server) = std::env::var("NETHERNODE_SNAPSHOT_SERVER") {
                        send(app, Message::StopServer(server));
                    }
                    45_000
                }
                "server-settings" => {
                    send(app, Message::EditServerSettings);
                    800
                }
                "friends" => {
                    send(app, Message::ShowFriends);
                    6000
                }
                "server-settings-save" => {
                    send(app, Message::EditServerSettings);
                    send(
                        app,
                        Message::ServerProperty("difficulty".into(), "hard".into()),
                    );
                    send(
                        app,
                        Message::ServerProperty("view-distance".into(), "12".into()),
                    );
                    send(
                        app,
                        Message::ServerProperty("hardcore".into(), "true".into()),
                    );
                    send(app, Message::SaveServerSettings);
                    800
                }
                "server-settings-all" => {
                    send(app, Message::EditServerSettings);
                    send(app, Message::ToggleAdvancedServerSettings);
                    800
                }
                "new-server-from" => {
                    send(app, Message::ShowNewServer);
                    send(app, Message::NewServerFrom(Some(selected.clone())));
                    4000
                }
                "new-server" => {
                    send(app, Message::ShowNewServer);
                    4000
                }
                "settings" => {
                    send(app, Message::ShowSettings);
                    800
                }
                "new-snapshot" => {
                    // Newest snapshot: most loaders haven't released for it yet.
                    let newest = app
                        .version_choices(VersionFilter::Snapshots)
                        .first()
                        .map(|c| c.id.clone());
                    send(app, Message::ShowNewInstance);
                    send(app, Message::NewFilter(VersionFilter::Snapshots));
                    if let Some(v) = newest {
                        send(app, Message::NewVersion(v));
                    }
                    6000
                }
                "new-all" => {
                    send(app, Message::ShowNewInstance);
                    send(app, Message::NewFilter(VersionFilter::All));
                    2500
                }
                "modpacks" => {
                    send(app, Message::ShowModpacks);
                    3500
                }
                "drop-pack" => {
                    let pack = std::env::var("NETHERNODE_SNAPSHOT_PACK").unwrap_or_default();
                    send(app, Message::ShowHome);
                    send(app, Message::FileDropped(pack.into()));
                    5000
                }
                "home-later" => {
                    send(app, Message::ShowHome);
                    150_000
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
