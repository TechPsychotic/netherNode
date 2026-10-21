// No console window behind the GUI on Windows (release builds only, so debug output still shows).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod game;
#[cfg(debug_assertions)]
mod snapshot;
mod theme;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use futures::channel::oneshot;
use iced::widget::{
    button, checkbox, column, combo_box, container, pick_list, progress_bar, row, rule, scrollable,
    space, text, text_input,
};
use iced::{Alignment, Element, Font, Length, Subscription, Task, window};
use nethernode_core::account::Accounts;
use nethernode_core::download;
use nethernode_core::instance::Instance;
use nethernode_core::memory::SystemMemory;
use nethernode_core::paths::Paths;
use nethernode_core::version::{self, ManifestEntry};

/// Lines of game output kept per instance; older lines are dropped to bound memory.
const LOG_LIMIT: usize = 2000;
/// Lines actually laid out in the console; laying out thousands of lines every frame is slow.
const LOG_VISIBLE: usize = 400;

pub fn main() -> iced::Result {
    // On low-memory machines draw with the CPU renderer: about 40 MB instead of about 90 MB
    // for the GPU one, which leaves more for the game. `ICED_BACKEND` still overrides it.
    if std::env::var_os("ICED_BACKEND").is_none() && SystemMemory::current().is_low() {
        // SAFETY: nothing else is running yet; this happens before any thread is spawned.
        unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
    }
    iced::application(App::new, App::update, App::view)
        .title(App::title)
        .theme(|_: &App| theme::theme())
        .subscription(App::subscription)
        .exit_on_close_request(false)
        .window_size((1040.0, 680.0))
        .run()
}

struct App {
    paths: Paths,
    instances: Vec<Instance>,
    selected: Option<String>,
    accounts: Accounts,
    screen: Screen,
    /// The version manifest, once it has loaded.
    versions: Option<Vec<ManifestEntry>>,
    runs: HashMap<String, Run>,
    logs: HashMap<String, VecDeque<String>>,
    notice: Option<(bool, String)>,
    /// Delete was clicked once for this instance; the second click deletes.
    confirm_delete: Option<String>,
    /// The window was closed while games were running; exit once they have stopped.
    closing: bool,
    memory: SystemMemory,
}

/// A launch in progress: downloading, then playing.
struct Run {
    stage: Option<(String, u64, u64)>,
    heap: Option<String>,
    stop: Option<oneshot::Sender<()>>,
}

enum Screen {
    Home,
    NewInstance(NewInstanceForm),
    EditInstance(EditForm),
    Accounts { name: String },
}

struct NewInstanceForm {
    name: String,
    version: Option<String>,
    snapshots: bool,
    low_end: bool,
    choices: combo_box::State<String>,
}

struct EditForm {
    original: String,
    auto_memory: bool,
    max_memory: String,
    java_path: String,
    jvm_args: String,
}

#[derive(Debug, Clone)]
enum Message {
    Select(String),
    Play(String),
    Stop(String),
    Game(String, game::Event),
    VersionsLoaded(Result<Vec<ManifestEntry>, String>),
    OpenFolder(PathBuf),
    Delete(String),
    DismissNotice,
    ShowHome,
    ShowNewInstance,
    NewName(String),
    NewVersion(String),
    NewSnapshots(bool),
    NewLowEnd(bool),
    CreateInstance,
    ShowEdit(String),
    EditAutoMemory(bool),
    EditMaxMemory(String),
    EditJava(String),
    EditJvmArgs(String),
    SaveEdit,
    ShowAccounts,
    AccountName(String),
    AddAccount,
    RemoveAccount(String),
    SetDefaultAccount(String),
    CloseRequested(window::Id),
    /// Debug builds only: screenshot each screen to `$NETHERNODE_SNAPSHOT` (see snapshot.rs).
    #[cfg(debug_assertions)]
    Snapshot(snapshot::Step),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let paths = Paths::new(Paths::default_root());
        let instances = Instance::list(&paths).unwrap_or_default();
        let accounts = Accounts::load(&paths).unwrap_or_default();
        let mut app = Self {
            selected: instances.first().map(|i| i.name.clone()),
            instances,
            accounts,
            screen: Screen::Home,
            versions: None,
            runs: HashMap::new(),
            logs: HashMap::new(),
            notice: None,
            confirm_delete: None,
            closing: false,
            memory: SystemMemory::current(),
            paths: paths.clone(),
        };
        if app.accounts.accounts.is_empty() {
            app.screen = Screen::Accounts {
                name: String::new(),
            };
            app.notice = Some((
                false,
                "Welcome! Add an offline account to get started.".into(),
            ));
        }
        let load = Task::perform(
            async move {
                version::manifest(&download::client(), &paths)
                    .await
                    .map(|m| m.versions)
                    .map_err(|e| e.to_string())
            },
            Message::VersionsLoaded,
        );
        #[cfg(debug_assertions)]
        let load = Task::batch([load, snapshot::start()]);
        (app, load)
    }

    fn title(&self) -> String {
        match self.runs.len() {
            0 => "netherNode".into(),
            n => format!("netherNode ({n} running)"),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        window::close_requests().map(Message::CloseRequested)
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.notice = Some((true, msg.into()));
    }

    fn reload_instances(&mut self) {
        self.instances = Instance::list(&self.paths).unwrap_or_default();
        if self
            .selected
            .as_ref()
            .is_none_or(|s| !self.instances.iter().any(|i| &i.name == s))
        {
            self.selected = self.instances.first().map(|i| i.name.clone());
        }
    }

    fn version_choices(&self, snapshots: bool) -> Vec<String> {
        self.versions
            .iter()
            .flatten()
            .filter(|v| v.kind == "release" || (snapshots && v.kind == "snapshot"))
            .map(|v| v.id.clone())
            .collect()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Select(name) => {
                self.selected = Some(name);
                self.confirm_delete = None;
                self.screen = Screen::Home;
            }

            Message::Play(name) => {
                if self.runs.contains_key(&name) {
                    return Task::none();
                }
                let Some(instance) = self.instances.iter().find(|i| i.name == name).cloned() else {
                    return Task::none();
                };
                let account = match self.accounts.pick(None) {
                    Ok(a) => a,
                    Err(e) => {
                        self.error(e.to_string());
                        self.screen = Screen::Accounts {
                            name: String::new(),
                        };
                        return Task::none();
                    }
                };
                let (stop_tx, stop_rx) = oneshot::channel();
                self.runs.insert(
                    name.clone(),
                    Run {
                        stage: None,
                        heap: None,
                        stop: Some(stop_tx),
                    },
                );
                let log = self.logs.entry(name.clone()).or_default();
                log.clear();
                log.push_back(format!(
                    "Launching {} as {}…",
                    instance.version, account.name
                ));
                self.notice = None;
                let stream = game::launch(self.paths.clone(), instance, account, stop_rx);
                return Task::run(stream, move |event| Message::Game(name.clone(), event));
            }

            Message::Stop(name) => {
                if let Some(stop) = self.runs.get_mut(&name).and_then(|r| r.stop.take()) {
                    let _ = stop.send(());
                }
            }

            Message::Game(name, event) => {
                let log = self.logs.entry(name.clone()).or_default();
                let mut push = |line: String| {
                    if log.len() >= LOG_LIMIT {
                        log.pop_front();
                    }
                    log.push_back(line);
                };
                match event {
                    game::Event::Stage { name: stage, total } => {
                        if let Some(run) = self.runs.get_mut(&name) {
                            run.stage = Some((stage, 0, total));
                        }
                    }
                    game::Event::Advance => {
                        if let Some((_, done, _)) =
                            self.runs.get_mut(&name).and_then(|r| r.stage.as_mut())
                        {
                            *done += 1;
                        }
                    }
                    game::Event::Started { heap, warning } => {
                        if let Some(w) = warning {
                            push(format!("Warning: {w}"));
                        }
                        push(format!("Game started (memory {heap})"));
                        if let Some(run) = self.runs.get_mut(&name) {
                            run.stage = None;
                            run.heap = Some(heap);
                        }
                    }
                    game::Event::Log(line) => push(line),
                    game::Event::Finished(result) => {
                        self.runs.remove(&name);
                        match result {
                            Ok(status) => push(status),
                            Err(e) => {
                                push(format!("Error: {e}"));
                                self.notice = Some((true, format!("{name}: {e}")));
                            }
                        }
                        if self.closing && self.runs.is_empty() {
                            return iced::exit();
                        }
                    }
                }
            }

            Message::VersionsLoaded(result) => match result {
                Ok(versions) => {
                    self.versions = Some(versions);
                    if let Screen::NewInstance(form) = &self.screen {
                        let choices = self.version_choices(form.snapshots);
                        if let Screen::NewInstance(form) = &mut self.screen {
                            form.version = form.version.take().or_else(|| choices.first().cloned());
                            form.choices = combo_box::State::new(choices);
                        }
                    }
                }
                Err(e) => self.error(format!("Couldn't load the version list: {e}")),
            },

            Message::OpenFolder(path) => open_folder(&path),

            Message::Delete(name) => {
                if self.confirm_delete.as_ref() != Some(&name) {
                    self.confirm_delete = Some(name);
                    return Task::none();
                }
                self.confirm_delete = None;
                match Instance::delete(&self.paths, &name) {
                    Ok(()) => {
                        self.logs.remove(&name);
                        self.reload_instances();
                        self.notice = Some((false, format!("Deleted {name}")));
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }

            Message::DismissNotice => self.notice = None,
            Message::ShowHome => self.screen = Screen::Home,

            Message::ShowNewInstance => {
                let choices = self.version_choices(false);
                self.screen = Screen::NewInstance(NewInstanceForm {
                    name: String::new(),
                    version: choices.first().cloned(),
                    snapshots: false,
                    low_end: self.memory.is_low(),
                    choices: combo_box::State::new(choices),
                });
            }
            Message::NewName(name) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.name = name;
                }
            }
            Message::NewVersion(v) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.version = Some(v);
                }
            }
            Message::NewSnapshots(on) => {
                let choices = self.version_choices(on);
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.snapshots = on;
                    f.choices = combo_box::State::new(choices);
                }
            }
            Message::NewLowEnd(on) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.low_end = on;
                }
            }
            Message::CreateInstance => {
                let Screen::NewInstance(f) = &self.screen else {
                    return Task::none();
                };
                let Some(version) = f.version.clone() else {
                    self.error("Pick a version first");
                    return Task::none();
                };
                let name = if f.name.trim().is_empty() {
                    version.clone()
                } else {
                    f.name.trim().to_string()
                };
                match Instance::create(&self.paths, &name, &version, f.low_end) {
                    Ok(_) => {
                        self.reload_instances();
                        self.selected = Some(name);
                        self.screen = Screen::Home;
                        self.notice = None;
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }

            Message::ShowEdit(name) => {
                if let Some(i) = self.instances.iter().find(|i| i.name == name) {
                    self.screen = Screen::EditInstance(EditForm {
                        original: i.name.clone(),
                        auto_memory: i.max_memory_mb.is_none(),
                        max_memory: i
                            .max_memory_mb
                            .map(|m| m.to_string())
                            .unwrap_or_else(|| self.memory.recommended_max_heap_mb().to_string()),
                        java_path: i
                            .java_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        jvm_args: i.jvm_args.join(" "),
                    });
                }
            }
            Message::EditAutoMemory(on) => {
                if let Screen::EditInstance(f) = &mut self.screen {
                    f.auto_memory = on;
                }
            }
            Message::EditMaxMemory(v) => {
                if let Screen::EditInstance(f) = &mut self.screen {
                    f.max_memory = v.chars().filter(char::is_ascii_digit).collect();
                }
            }
            Message::EditJava(v) => {
                if let Screen::EditInstance(f) = &mut self.screen {
                    f.java_path = v;
                }
            }
            Message::EditJvmArgs(v) => {
                if let Screen::EditInstance(f) = &mut self.screen {
                    f.jvm_args = v;
                }
            }
            Message::SaveEdit => {
                let Screen::EditInstance(f) = &self.screen else {
                    return Task::none();
                };
                let Some(mut instance) = self
                    .instances
                    .iter()
                    .find(|i| i.name == f.original)
                    .cloned()
                else {
                    return Task::none();
                };
                instance.max_memory_mb = if f.auto_memory {
                    None
                } else {
                    match f.max_memory.parse::<u32>() {
                        Ok(mb) if mb >= 512 => Some(mb),
                        _ => {
                            self.error("Memory must be a number of MB, at least 512");
                            return Task::none();
                        }
                    }
                };
                let java = f.java_path.trim();
                instance.java_path = (!java.is_empty()).then(|| PathBuf::from(java));
                instance.jvm_args = f.jvm_args.split_whitespace().map(String::from).collect();
                match instance.save(&self.paths) {
                    Ok(()) => {
                        self.reload_instances();
                        self.screen = Screen::Home;
                        self.notice = Some((false, "Saved".into()));
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }

            Message::ShowAccounts => {
                self.screen = Screen::Accounts {
                    name: String::new(),
                }
            }
            Message::AccountName(v) => {
                if let Screen::Accounts { name } = &mut self.screen {
                    *name = v;
                }
            }
            Message::AddAccount => {
                let Screen::Accounts { name } = &mut self.screen else {
                    return Task::none();
                };
                let result = self
                    .accounts
                    .add(name.trim())
                    .and_then(|_| self.accounts.save(&self.paths));
                match result {
                    Ok(()) => {
                        name.clear();
                        self.notice = None;
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }
            Message::RemoveAccount(name) => {
                if let Err(e) = self
                    .accounts
                    .remove(&name)
                    .and_then(|_| self.accounts.save(&self.paths))
                {
                    self.error(e.to_string());
                }
            }
            Message::SetDefaultAccount(name) => {
                if let Err(e) = self
                    .accounts
                    .set_default(&name)
                    .and_then(|_| self.accounts.save(&self.paths))
                {
                    self.error(e.to_string());
                }
            }

            #[cfg(debug_assertions)]
            Message::Snapshot(step) => return snapshot::update(self, step),

            Message::CloseRequested(id) => {
                if self.runs.is_empty() {
                    return window::close(id).chain(iced::exit());
                }
                // Never leave a game running unseen: stop everything, exit once all have ended.
                self.closing = true;
                for run in self.runs.values_mut() {
                    if let Some(stop) = run.stop.take() {
                        let _ = stop.send(());
                    }
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let main = match &self.screen {
            Screen::Home => self.view_home(),
            Screen::NewInstance(form) => self.view_new_instance(form),
            Screen::EditInstance(form) => self.view_edit(form),
            Screen::Accounts { name } => self.view_accounts(name),
        };
        let mut content = column![].spacing(16).padding(24).height(Length::Fill);
        if let Some((is_error, msg)) = &self.notice {
            content = content.push(
                container(
                    row![
                        text(msg).width(Length::Fill),
                        button(text("✕").size(12))
                            .style(button::text)
                            .on_press(Message::DismissNotice),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding([8, 14])
                .style(theme::notice(*is_error)),
            );
        }
        content = content.push(main);
        row![self.view_sidebar(), content].into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let mut list = column![].spacing(4);
        for instance in &self.instances {
            let selected = self.selected.as_ref() == Some(&instance.name)
                && matches!(self.screen, Screen::Home);
            let status = if self.runs.contains_key(&instance.name) {
                "● "
            } else {
                ""
            };
            list = list.push(
                button(column![
                    text(format!("{status}{}", instance.name)).size(15),
                    text(&instance.version).size(12).color(theme::MUTED),
                ])
                .width(Length::Fill)
                .padding([8, 12])
                .style(theme::list_item(selected))
                .on_press(Message::Select(instance.name.clone())),
            );
        }
        if self.instances.is_empty() {
            list = list.push(text("No instances yet").size(13).color(theme::MUTED));
        }

        let names: Vec<String> = self
            .accounts
            .accounts
            .iter()
            .map(|a| a.name.clone())
            .collect();
        let account = column![
            text("Playing as").size(12).color(theme::MUTED),
            row![
                pick_list(
                    names,
                    self.accounts.default.clone(),
                    Message::SetDefaultAccount
                )
                .placeholder("No account")
                .width(Length::Fill),
                button(text("⚙").size(14))
                    .style(theme::secondary)
                    .padding([6, 10])
                    .on_press(Message::ShowAccounts),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(6);

        container(
            column![
                text("netherNode").size(22).color(theme::ACCENT),
                text("Offline Minecraft launcher")
                    .size(12)
                    .color(theme::MUTED),
                rule::horizontal(1),
                scrollable(list).height(Length::Fill),
                button(
                    text("+  New instance")
                        .width(Length::Fill)
                        .align_x(Alignment::Center)
                )
                .width(Length::Fill)
                .padding(10)
                .style(theme::secondary)
                .on_press(Message::ShowNewInstance),
                account,
            ]
            .spacing(12)
            .padding(16),
        )
        .width(240)
        .height(Length::Fill)
        .style(theme::sidebar)
        .into()
    }

    fn view_home(&self) -> Element<'_, Message> {
        let Some(instance) = self
            .selected
            .as_ref()
            .and_then(|s| self.instances.iter().find(|i| &i.name == s))
        else {
            return container(
                column![
                    text("No instance selected").size(20),
                    text("Create an instance to pick a Minecraft version and play.")
                        .color(theme::MUTED),
                    button(text("+  New instance"))
                        .padding([10, 18])
                        .style(theme::play)
                        .on_press(Message::ShowNewInstance),
                ]
                .spacing(12)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into();
        };

        let run = self.runs.get(&instance.name);
        let memory = match (run.and_then(|r| r.heap.clone()), instance.max_memory_mb) {
            (Some(heap), _) => format!("Memory {heap}"),
            (None, Some(mb)) => format!("Memory up to {mb} MB"),
            (None, None) => "Memory: automatic".into(),
        };
        let action = match run {
            Some(r) if r.stop.is_none() => button(text("Stopping…").size(18))
                .padding([12, 36])
                .style(theme::stop),
            Some(_) => button(text("■  Stop").size(18))
                .padding([12, 36])
                .style(theme::stop)
                .on_press(Message::Stop(instance.name.clone())),
            None => button(text("▶  Play").size(18))
                .padding([12, 40])
                .style(theme::play)
                .on_press_maybe(
                    (!self.accounts.accounts.is_empty())
                        .then(|| Message::Play(instance.name.clone())),
                ),
        };
        let deleting = self.confirm_delete.as_ref() == Some(&instance.name);
        let header = container(
            row![
                column![
                    text(&instance.name).size(26),
                    text(format!("Minecraft {}  ·  {memory}", instance.version))
                        .color(theme::MUTED),
                ]
                .spacing(4)
                .width(Length::Fill),
                button(text("Edit"))
                    .style(theme::secondary)
                    .padding([8, 14])
                    .on_press_maybe(
                        run.is_none()
                            .then(|| Message::ShowEdit(instance.name.clone()))
                    ),
                button(text("Folder"))
                    .style(theme::secondary)
                    .padding([8, 14])
                    .on_press(Message::OpenFolder(instance.game_dir(&self.paths))),
                button(text(if deleting { "Really delete?" } else { "Delete" }))
                    .style(theme::danger_outline)
                    .padding([8, 14])
                    .on_press_maybe(
                        run.is_none()
                            .then(|| Message::Delete(instance.name.clone()))
                    ),
                action,
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding(20)
        .style(theme::card);

        let mut body = column![header].spacing(16);
        if let Some((stage, done, total)) = run.and_then(|r| r.stage.as_ref()) {
            body = body.push(
                column![
                    text(format!("{stage}: {done} / {total}"))
                        .size(13)
                        .color(theme::MUTED),
                    progress_bar(0.0..=(*total).max(1) as f32, *done as f32).girth(8),
                ]
                .spacing(6),
            );
        }

        let lines = self.logs.get(&instance.name).filter(|l| !l.is_empty());
        if lines.is_none() {
            body = body.push(
                container(
                    text("Game output will appear here when you press Play.").color(theme::MUTED),
                )
                .center(Length::Fill)
                .style(theme::console),
            );
            return body.into();
        }
        let shown = lines.map_or(0, |l| l.len().min(LOG_VISIBLE));
        let console = column(
            lines
                .into_iter()
                .flat_map(|l| l.iter().skip(l.len() - shown))
                .map(|line| text(line).size(12).font(Font::MONOSPACE).into()),
        )
        .spacing(2);
        body = body.push(
            container(
                scrollable(container(console).padding(12).width(Length::Fill))
                    .anchor_bottom()
                    .height(Length::Fill),
            )
            .style(theme::console)
            .height(Length::Fill),
        );
        body.into()
    }

    fn view_new_instance<'a>(&'a self, form: &'a NewInstanceForm) -> Element<'a, Message> {
        let versions: Element<'a, Message> = if self.versions.is_some() {
            combo_box(
                &form.choices,
                "Search versions…",
                form.version.as_ref(),
                Message::NewVersion,
            )
            .input_style(theme::input)
            .padding(10)
            .into()
        } else {
            text("Loading versions…").color(theme::MUTED).into()
        };
        let can_create = form.version.is_some();
        let card = container(
            column![
                text("New instance").size(24),
                field(
                    "Name",
                    text_input(form.version.as_deref().unwrap_or("My world"), &form.name)
                        .on_input(Message::NewName)
                        .on_submit_maybe(can_create.then_some(Message::CreateInstance))
                        .style(theme::input)
                        .padding(10)
                ),
                field("Minecraft version", versions),
                checkbox(form.snapshots)
                    .label("Show snapshots")
                    .on_toggle(Message::NewSnapshots),
                checkbox(form.low_end)
                    .label(
                        "Low-end graphics defaults (render distance 6, 60 FPS cap, fast graphics)"
                    )
                    .on_toggle(Message::NewLowEnd),
                row![
                    space::horizontal(),
                    button(text("Cancel"))
                        .style(theme::secondary)
                        .padding([10, 18])
                        .on_press(Message::ShowHome),
                    button(text("Create"))
                        .style(theme::play)
                        .padding([10, 22])
                        .on_press_maybe(can_create.then_some(Message::CreateInstance)),
                ]
                .spacing(10),
            ]
            .spacing(18),
        )
        .padding(24)
        .max_width(560)
        .style(theme::card);
        container(card).center_x(Length::Fill).into()
    }

    fn view_edit<'a>(&'a self, form: &'a EditForm) -> Element<'a, Message> {
        let memory_note = format!(
            "This computer has {} MB of RAM. Automatic uses about a quarter of it ({} MB) and shrinks if less is free.",
            self.memory.total_mb,
            self.memory.recommended_max_heap_mb()
        );
        let mut memory = column![
            checkbox(form.auto_memory)
                .label("Automatic memory")
                .on_toggle(Message::EditAutoMemory),
            text(memory_note).size(12).color(theme::MUTED),
        ]
        .spacing(8);
        if !form.auto_memory {
            memory = memory.push(field(
                "Maximum memory (MB)",
                text_input("2048", &form.max_memory)
                    .on_input(Message::EditMaxMemory)
                    .style(theme::input)
                    .padding(10),
            ));
        }
        let card = container(
            column![
                text(format!("Edit {}", form.original)).size(24),
                memory,
                field(
                    "Java executable (leave empty to pick automatically)",
                    text_input("Automatic", &form.java_path)
                        .on_input(Message::EditJava)
                        .style(theme::input)
                        .padding(10),
                ),
                field(
                    "Extra JVM arguments",
                    text_input("e.g. -XX:+UseZGC", &form.jvm_args)
                        .on_input(Message::EditJvmArgs)
                        .style(theme::input)
                        .padding(10),
                ),
                row![
                    space::horizontal(),
                    button(text("Cancel"))
                        .style(theme::secondary)
                        .padding([10, 18])
                        .on_press(Message::ShowHome),
                    button(text("Save"))
                        .style(theme::play)
                        .padding([10, 22])
                        .on_press(Message::SaveEdit),
                ]
                .spacing(10),
            ]
            .spacing(18),
        )
        .padding(24)
        .max_width(560)
        .style(theme::card);
        container(card).center_x(Length::Fill).into()
    }

    fn view_accounts<'a>(&'a self, name: &'a str) -> Element<'a, Message> {
        let mut list = column![].spacing(8);
        for account in &self.accounts.accounts {
            let is_default = self.accounts.default.as_ref() == Some(&account.name);
            list = list.push(
                container(
                    row![
                        column![
                            text(&account.name).size(16),
                            text(&account.uuid)
                                .size(11)
                                .font(Font::MONOSPACE)
                                .color(theme::MUTED),
                        ]
                        .spacing(2)
                        .width(Length::Fill),
                        if is_default {
                            Element::from(text("Default").size(13).color(theme::SUCCESS))
                        } else {
                            button(text("Make default"))
                                .style(theme::secondary)
                                .padding([6, 12])
                                .on_press(Message::SetDefaultAccount(account.name.clone()))
                                .into()
                        },
                        button(text("Remove"))
                            .style(theme::danger_outline)
                            .padding([6, 12])
                            .on_press(Message::RemoveAccount(account.name.clone())),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding(12)
                .style(theme::console),
            );
        }
        let can_add = !name.trim().is_empty();
        let card = container(
            column![
                text("Accounts").size(24),
                text("Offline accounts only: no login, no internet check. Your name and UUID are what offline servers and LAN worlds see.")
                    .size(13)
                    .color(theme::MUTED),
                list,
                row![
                    text_input("Username (3–16 letters, digits or _)", name)
                        .on_input(Message::AccountName)
                        .on_submit_maybe(can_add.then_some(Message::AddAccount))
                        .style(theme::input)
                        .padding(10),
                    button(text("Add")).style(theme::play).padding([10, 20]).on_press_maybe(can_add.then_some(Message::AddAccount)),
                ]
                .spacing(10),
                row![
                    space::horizontal(),
                    button(text("Done"))
                        .style(theme::secondary)
                        .padding([10, 18])
                        .on_press_maybe((!self.accounts.accounts.is_empty()).then_some(Message::ShowHome)),
                ],
            ]
            .spacing(18),
        )
        .padding(24)
        .max_width(620)
        .style(theme::card);
        container(card).center_x(Length::Fill).into()
    }
}

fn field<'a>(label: &'a str, input: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![text(label).size(13).color(theme::MUTED), input.into()]
        .spacing(6)
        .into()
}

fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(path).spawn();
}
