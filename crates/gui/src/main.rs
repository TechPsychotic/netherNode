// No console window behind the GUI on Windows (release builds only, so debug output still shows).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod background;
mod game;
mod icon;
#[cfg(debug_assertions)]
mod snapshot;
mod textures;
mod theme;
mod ui;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use futures::channel::oneshot;
use iced::{Subscription, Task, window};
use nethernode_core::account::Accounts;
use nethernode_core::crash::CrashInfo;
use nethernode_core::download;
use nethernode_core::instance::Instance;
use nethernode_core::loader::{self, LoaderKind, LoaderVersion};
use nethernode_core::memory::SystemMemory;
use nethernode_core::modpack;
use nethernode_core::mods::{self, InstalledMod, SearchHit};
use nethernode_core::paths::Paths;
use nethernode_core::version::{self, ManifestEntry};

use textures::GameTextures;

/// Lines of game output kept per instance; older lines are dropped to bound memory.
const LOG_LIMIT: usize = 2000;

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
        .window(window::Settings {
            size: iced::Size::new(1180.0, 760.0),
            min_size: Some(iced::Size::new(900.0, 600.0)),
            icon: icon::window_icon(),
            // We stop running games before exiting; see Message::CloseRequested.
            exit_on_close_request: false,
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                // Must match the .desktop file name so docks show our icon on Wayland.
                application_id: "nethernode".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

/// GUI preferences, saved as `gui.json` in the data folder.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Prefs {
    animated_background: bool,
}

impl Prefs {
    fn load(paths: &Paths) -> Self {
        std::fs::read(paths.root.join("gui.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Self {
                animated_background: true,
            })
    }

    fn save(&self, paths: &Paths) {
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::create_dir_all(&paths.root);
            let _ = std::fs::write(paths.root.join("gui.json"), bytes);
        }
    }
}

pub struct App {
    paths: Paths,
    instances: Vec<Instance>,
    selected: Option<String>,
    accounts: Accounts,
    screen: Screen,
    /// The version manifest, once it has loaded.
    versions: Option<Vec<ManifestEntry>>,
    runs: HashMap<String, Run>,
    /// Loader installs and other long jobs, per instance.
    jobs: HashMap<String, Job>,
    logs: HashMap<String, VecDeque<String>>,
    notice: Option<(bool, String)>,
    /// Delete was clicked once for this instance; the second click deletes.
    confirm_delete: Option<String>,
    /// The window was closed while games were running; exit once they have stopped.
    closing: bool,
    memory: SystemMemory,
    /// The last crash of each instance, shown until dismissed or the next launch.
    crashes: HashMap<String, CrashInfo>,
    logo: iced::widget::image::Handle,
    textures: Option<Arc<GameTextures>>,
    prefs: Prefs,
    /// Shaders need the GPU renderer; the CPU one (low-memory mode) gets a static backdrop.
    gpu: bool,
    hovered: Option<String>,
    /// The version list is being re-fetched from Mojang.
    refreshing_versions: bool,
    /// Modpack installs in progress, by Modrinth project id or file path.
    pack_jobs: HashMap<String, Job>,
}

/// A launch in progress: downloading, then playing.
pub struct Run {
    stage: Option<(String, u64, u64)>,
    heap: Option<String>,
    stop: Option<oneshot::Sender<()>>,
}

pub struct Job {
    label: String,
    stage: Option<(String, u64, u64)>,
}

pub enum Screen {
    Home,
    NewInstance(NewInstanceForm),
    EditInstance(EditForm),
    Accounts { name: String },
    Mods(ModsView),
    Modpacks(PacksView),
}

/// Something loaded in the background.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

/// The loader choice shared by the new-instance and edit forms.
pub struct LoaderPick {
    kind: Option<LoaderKind>,
    versions: Option<Load<Vec<LoaderVersion>>>,
    version: Option<String>,
    optifine_jar: Option<PathBuf>,
}

pub struct NewInstanceForm {
    name: String,
    version: Option<String>,
    filter: VersionFilter,
    low_end: bool,
    loader: LoaderPick,
}

/// Which Minecraft versions the dropdown lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionFilter {
    Releases,
    Snapshots,
    /// Everything, including the old alphas and betas.
    All,
}

/// One entry of the version dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionChoice {
    pub id: String,
    pub kind: String,
    /// Already downloaded, so it starts without a download.
    pub downloaded: bool,
}

impl std::fmt::Display for VersionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind.as_str() {
            "release" => "",
            "snapshot" => "  · snapshot",
            "old_beta" => "  · beta",
            "old_alpha" => "  · alpha",
            _ => "",
        };
        let downloaded = if self.downloaded {
            "  ✓ downloaded"
        } else {
            ""
        };
        write!(f, "{}{kind}{downloaded}", self.id)
    }
}

/// Where a modpack comes from.
#[derive(Debug, Clone)]
enum PackSource {
    Modrinth(String),
    File(PathBuf),
}

pub struct PacksView {
    query: String,
    results: Option<Load<Vec<SearchHit>>>,
    total: u64,
}

pub struct EditForm {
    original: String,
    auto_memory: bool,
    max_memory: String,
    java_path: String,
    jvm_args: String,
    loader: LoaderPick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModsTab {
    Installed,
    Browse,
}

pub struct ModsView {
    instance: String,
    tab: ModsTab,
    installed: Vec<InstalledMod>,
    query: String,
    results: Option<Load<Vec<SearchHit>>>,
    total: u64,
    /// Project ids being installed right now.
    installing: HashSet<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Select(String),
    Hover(Option<String>),
    Play(String),
    Stop(String),
    Game(String, game::Event),
    Job(String, game::JobEvent),
    VersionsLoaded(Result<Vec<ManifestEntry>, String>),
    TexturesLoaded(Option<Arc<GameTextures>>),
    ToggleAnimation,
    OpenFolder(PathBuf),
    OpenFile(PathBuf),
    OpenUrl(String),
    CopyCrash(String),
    DismissCrash(String),
    Delete(String),
    DismissNotice,
    ShowHome,
    ShowNewInstance,
    NewName(String),
    NewVersion(String),
    NewFilter(VersionFilter),
    RefreshVersions,
    NewLowEnd(bool),
    CreateInstance,
    ShowEdit(String),
    EditAutoMemory(bool),
    EditMaxMemory(String),
    EditJava(String),
    EditJvmArgs(String),
    SaveEdit,
    PickLoader(Option<LoaderKind>),
    LoaderVersions(LoaderKind, String, Result<Vec<LoaderVersion>, String>),
    PickLoaderVersion(String),
    BrowseOptiFine,
    OptiFinePicked(Option<PathBuf>),
    ShowAccounts,
    AccountName(String),
    AddAccount,
    RemoveAccount(String),
    SetDefaultAccount(String),
    ShowMods(String),
    ModsTab(ModsTab),
    ModsQuery(String),
    ModsSearch,
    ModsMore,
    ModsResults(u32, Result<mods::SearchResults, String>),
    InstallMod(String),
    ModInstalled(String, Result<Vec<String>, String>),
    ToggleMod(String, bool),
    RemoveMod(String),
    AddModFiles,
    ModFilesPicked(Vec<PathBuf>),
    FileDropped(PathBuf),
    ShowModpacks,
    PacksQuery(String),
    PacksSearch,
    PacksMore,
    PacksResults(u32, Result<mods::SearchResults, String>),
    InstallPack(String),
    ImportPack,
    PackPicked(Option<PathBuf>),
    PackJob(String, game::JobEvent),
    CloseRequested(window::Id),
    /// Debug builds only: screenshot each screen to `$NETHERNODE_SNAPSHOT` (see snapshot.rs).
    #[cfg(debug_assertions)]
    Snapshot(snapshot::Step),
}

/// Fetch Mojang's current version list (falls back to the cached copy when offline).
fn load_versions(paths: Paths) -> Task<Message> {
    Task::perform(
        async move {
            version::manifest(&download::client(), &paths)
                .await
                .map(|m| m.versions)
                .map_err(|e| e.to_string())
        },
        Message::VersionsLoaded,
    )
}

fn load_textures(paths: Paths) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || textures::load(&paths).map(Arc::new))
                .await
                .ok()
                .flatten()
        },
        Message::TexturesLoaded,
    )
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let paths = Paths::new(Paths::default_root());
        let interrupted = Instance::clean_interrupted(&paths);
        let instances = Instance::list(&paths).unwrap_or_default();
        let accounts = Accounts::load(&paths).unwrap_or_default();
        let gpu = std::env::var("ICED_BACKEND").map_or(true, |b| !b.starts_with("tiny-skia"));
        let mut app = Self {
            selected: instances.first().map(|i| i.name.clone()),
            instances,
            accounts,
            screen: Screen::Home,
            versions: None,
            runs: HashMap::new(),
            jobs: HashMap::new(),
            logs: HashMap::new(),
            notice: None,
            confirm_delete: None,
            closing: false,
            memory: SystemMemory::current(),
            crashes: HashMap::new(),
            logo: icon::logo(),
            textures: None,
            prefs: Prefs::load(&paths),
            gpu,
            hovered: None,
            refreshing_versions: true,
            pack_jobs: HashMap::new(),
            paths: paths.clone(),
        };
        if !interrupted.is_empty() {
            app.notice = Some((
                false,
                format!(
                    "Removed unfinished install(s) from last time: {}. Install them again from Modpacks.",
                    interrupted.join(", ")
                ),
            ));
        }
        if app.accounts.accounts.is_empty() {
            app.screen = Screen::Accounts {
                name: String::new(),
            };
            app.notice = Some((
                false,
                "Welcome to netherNode! Add an offline account to get started.".into(),
            ));
        }
        let tasks = Task::batch([load_versions(paths.clone()), load_textures(paths)]);
        #[cfg(debug_assertions)]
        let tasks = Task::batch([tasks, snapshot::start()]);
        (app, tasks)
    }

    fn title(&self) -> String {
        match self.runs.len() {
            0 => "netherNode".into(),
            n => format!("netherNode ({n} running)"),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let drops = iced::event::listen_with(|event, _, _| match event {
            iced::Event::Window(window::Event::FileDropped(path)) => {
                Some(Message::FileDropped(path))
            }
            _ => None,
        });
        Subscription::batch([window::close_requests().map(Message::CloseRequested), drops])
    }

    /// The background animates unless turned off, unsupported, or a game needs the GPU.
    fn animating(&self) -> bool {
        self.gpu && self.prefs.animated_background && self.runs.values().all(|r| r.heap.is_none())
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.notice = Some((true, msg.into()));
    }

    fn instance(&self, name: &str) -> Option<&Instance> {
        self.instances.iter().find(|i| i.name == name)
    }

    fn reload_instances(&mut self) {
        self.instances = Instance::list(&self.paths).unwrap_or_default();
        if self
            .selected
            .as_ref()
            .is_none_or(|s| self.instance(s).is_none())
        {
            self.selected = self.instances.first().map(|i| i.name.clone());
        }
    }

    /// Every version Mojang lists that matches `filter`, newest first, marking downloaded ones.
    pub fn version_choices(&self, filter: VersionFilter) -> Vec<VersionChoice> {
        self.versions
            .iter()
            .flatten()
            .filter(|v| match filter {
                VersionFilter::Releases => v.kind == "release",
                VersionFilter::Snapshots => v.kind == "release" || v.kind == "snapshot",
                VersionFilter::All => true,
            })
            .map(|v| VersionChoice {
                id: v.id.clone(),
                kind: v.kind.clone(),
                downloaded: self.paths.version_jar(&v.id).exists(),
            })
            .collect()
    }

    fn packs_view(&mut self) -> Option<&mut PacksView> {
        match &mut self.screen {
            Screen::Modpacks(view) => Some(view),
            _ => None,
        }
    }

    fn search_packs(&mut self, offset: u32) -> Task<Message> {
        let Some(view) = self.packs_view() else {
            return Task::none();
        };
        let query = view.query.clone();
        if offset == 0 {
            view.results = Some(Load::Loading);
        }
        Task::perform(
            async move {
                modpack::search(&download::client(), &query, offset)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |result| Message::PacksResults(offset, result),
        )
    }

    /// Install or import a modpack as a background job; it becomes a new instance.
    fn start_pack_job(&mut self, source: PackSource, label: String) -> Task<Message> {
        let key = match &source {
            PackSource::Modrinth(id) => id.clone(),
            PackSource::File(path) => path.display().to_string(),
        };
        if self.pack_jobs.contains_key(&key) {
            return Task::none();
        }
        self.notice = Some((
            false,
            format!("Installing {label}… it will appear in your instances when it's ready."),
        ));
        self.pack_jobs
            .insert(key.clone(), Job { label, stage: None });
        let paths = self.paths.clone();
        let stream = game::job(move |progress| async move {
            let http = download::client();
            let result = match source {
                PackSource::Modrinth(id) => modpack::install(&http, &paths, &id, &progress).await,
                PackSource::File(path) => {
                    modpack::import(&http, &paths, &path, None, None, &progress).await
                }
            };
            result
                .map(|instance| instance.name)
                .map_err(|e| e.to_string())
        });
        Task::run(stream, move |event| Message::PackJob(key.clone(), event))
    }

    /// The Minecraft version the loader picker on the current screen applies to.
    fn picker_minecraft(&self) -> Option<String> {
        match &self.screen {
            Screen::NewInstance(f) => f.version.clone(),
            Screen::EditInstance(f) => self
                .instance(&f.original)
                .map(|i| i.minecraft_version().to_string()),
            _ => None,
        }
    }

    fn picker_mut(&mut self) -> Option<&mut LoaderPick> {
        match &mut self.screen {
            Screen::NewInstance(f) => Some(&mut f.loader),
            Screen::EditInstance(f) => Some(&mut f.loader),
            _ => None,
        }
    }

    /// Fetch loader versions for the picker's current choice.
    fn load_loader_versions(&mut self) -> Task<Message> {
        let Some(mc) = self.picker_minecraft() else {
            return Task::none();
        };
        let Some(pick) = self.picker_mut() else {
            return Task::none();
        };
        let Some(kind) = pick.kind.filter(|k| *k != LoaderKind::OptiFine) else {
            pick.versions = None;
            return Task::none();
        };
        pick.versions = Some(Load::Loading);
        Task::perform(
            async move {
                let result = loader::versions(&download::client(), kind, &mc)
                    .await
                    .map_err(|e| e.to_string());
                (kind, mc, result)
            },
            |(kind, mc, result)| Message::LoaderVersions(kind, mc, result),
        )
    }

    /// Install the picked loader on `instance` as a background job.
    fn start_loader_job(&mut self, instance: Instance, pick: &LoaderPick) -> Task<Message> {
        let Some(kind) = pick.kind else {
            return Task::none();
        };
        let name = instance.name.clone();
        let paths = self.paths.clone();
        let version = pick.version.clone();
        let jar = pick.optifine_jar.clone();
        self.jobs.insert(
            name.clone(),
            Job {
                label: format!("Installing {kind}"),
                stage: None,
            },
        );
        let stream = game::job(move |progress| async move {
            let http = download::client();
            let mut instance = instance;
            let result = match (kind, jar) {
                (LoaderKind::OptiFine, Some(jar)) => {
                    loader::apply_optifine(&http, &paths, &mut instance, &jar, &progress).await
                }
                (LoaderKind::OptiFine, None) => Err(nethernode_core::Error::Other(
                    "choose the OptiFine jar first".into(),
                )),
                _ => {
                    loader::apply(
                        &http,
                        &paths,
                        &mut instance,
                        kind,
                        version.as_deref(),
                        &progress,
                    )
                    .await
                }
            };
            result
                .map(|()| format!("{kind} installed"))
                .map_err(|e| e.to_string())
        });
        Task::run(stream, move |event| Message::Job(name.clone(), event))
    }

    fn mods_view(&mut self) -> Option<&mut ModsView> {
        match &mut self.screen {
            Screen::Mods(view) => Some(view),
            _ => None,
        }
    }

    /// The instance whose Mods screen is open.
    fn mods_instance(&mut self) -> Option<Instance> {
        let name = self.mods_view()?.instance.clone();
        self.instance(&name).cloned()
    }

    fn refresh_mods(&mut self) {
        let Some(instance) = self.mods_instance() else {
            return;
        };
        let installed = mods::list(&self.paths, &instance).unwrap_or_default();
        if let Some(view) = self.mods_view() {
            view.installed = installed;
        }
    }

    fn search_mods(&mut self, offset: u32) -> Task<Message> {
        let Some(instance) = self.mods_instance() else {
            return Task::none();
        };
        let Some(view) = self.mods_view() else {
            return Task::none();
        };
        let query = view.query.clone();
        if offset == 0 {
            view.results = Some(Load::Loading);
        }
        Task::perform(
            async move {
                mods::search(&download::client(), &instance, &query, offset)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |result| Message::ModsResults(offset, result),
        )
    }

    fn add_mod_file(&mut self, path: &std::path::Path) {
        let Some(instance) = self.mods_instance() else {
            self.error("Open an instance's Mods screen first, then drop mod files on the window");
            return;
        };
        match mods::add_file(&self.paths, &instance, path) {
            Ok(file) => self.notice = Some((false, format!("Added {file}"))),
            Err(e) => self.error(e.to_string()),
        }
        self.refresh_mods();
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Select(name) => {
                self.selected = Some(name);
                self.confirm_delete = None;
                self.screen = Screen::Home;
            }
            Message::Hover(name) => self.hovered = name,

            Message::Play(name) => {
                if self.runs.contains_key(&name) || self.jobs.contains_key(&name) {
                    return Task::none();
                }
                let Some(instance) = self.instance(&name).cloned() else {
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
                self.crashes.remove(&name);
                self.notice = None;
                self.selected = Some(name.clone());
                self.screen = Screen::Home;
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
                    game::Event::Finished(ended) => {
                        self.runs.remove(&name);
                        match ended {
                            game::Ended::Ok(status) => push(status),
                            game::Ended::Failed(e) => {
                                push(format!("Error: {e}"));
                                self.notice = Some((true, format!("{name}: {e}")));
                            }
                            game::Ended::Crashed(crash) => {
                                push(crash.title.clone());
                                self.crashes.insert(name, crash);
                            }
                        }
                        if self.closing && self.runs.is_empty() {
                            return iced::exit();
                        }
                        // The first launch may have downloaded the textures we were missing.
                        if self.textures.is_none() {
                            return load_textures(self.paths.clone());
                        }
                    }
                }
            }

            Message::Job(name, event) => match event {
                game::JobEvent::Stage { name: stage, total } => {
                    if let Some(job) = self.jobs.get_mut(&name) {
                        job.stage = Some((stage, 0, total));
                    }
                }
                game::JobEvent::Advance => {
                    if let Some((_, done, _)) =
                        self.jobs.get_mut(&name).and_then(|j| j.stage.as_mut())
                    {
                        *done += 1;
                    }
                }
                game::JobEvent::Done(result) => {
                    self.jobs.remove(&name);
                    self.reload_instances();
                    match result {
                        Ok(msg) => self.notice = Some((false, format!("{name}: {msg}"))),
                        Err(e) => self.error(format!("{name}: {e}")),
                    }
                    if self.textures.is_none() {
                        return load_textures(self.paths.clone());
                    }
                }
            },

            Message::VersionsLoaded(result) => {
                self.refreshing_versions = false;
                match result {
                    Ok(versions) => {
                        self.versions = Some(versions);
                        // A form opened before the list arrived gets the newest release.
                        let newest = self
                            .version_choices(VersionFilter::Releases)
                            .first()
                            .map(|c| c.id.clone());
                        if let Screen::NewInstance(form) = &mut self.screen
                            && form.version.is_none()
                        {
                            form.version = newest;
                            return self.load_loader_versions();
                        }
                    }
                    Err(e) => self.error(format!("Couldn't load the version list: {e}")),
                }
            }
            Message::RefreshVersions => {
                self.refreshing_versions = true;
                return load_versions(self.paths.clone());
            }

            Message::TexturesLoaded(textures) => {
                if textures.is_some() {
                    self.textures = textures;
                }
            }
            Message::ToggleAnimation => {
                self.prefs.animated_background = !self.prefs.animated_background;
                self.prefs.save(&self.paths);
            }

            Message::OpenFolder(path) => {
                let _ = std::fs::create_dir_all(&path);
                open_path(path.as_os_str());
            }
            Message::OpenFile(path) => open_path(path.as_os_str()),
            Message::OpenUrl(url) => open_path(std::ffi::OsStr::new(&url)),
            Message::CopyCrash(name) => {
                if let Some(crash) = self.crashes.get(&name) {
                    return iced::clipboard::write(ui::crash_text(&name, crash));
                }
            }
            Message::DismissCrash(name) => {
                self.crashes.remove(&name);
            }

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
                let newest = self
                    .version_choices(VersionFilter::Releases)
                    .first()
                    .map(|c| c.id.clone());
                self.screen = Screen::NewInstance(NewInstanceForm {
                    name: String::new(),
                    version: newest,
                    filter: VersionFilter::Releases,
                    low_end: self.memory.is_low(),
                    loader: LoaderPick {
                        kind: None,
                        versions: None,
                        version: None,
                        optifine_jar: None,
                    },
                });
                // Always offer Mojang's current list, not just what was cached at startup.
                self.refreshing_versions = true;
                return load_versions(self.paths.clone());
            }
            Message::NewName(name) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.name = name;
                }
            }
            Message::NewVersion(v) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.version = Some(v);
                    f.loader.optifine_jar = None;
                }
                // Loader versions depend on the Minecraft version.
                return self.load_loader_versions();
            }
            Message::NewFilter(filter) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    f.filter = filter;
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
                    self.error("Pick a Minecraft version first");
                    return Task::none();
                };
                if f.loader.kind == Some(LoaderKind::OptiFine) && f.loader.optifine_jar.is_none() {
                    self.error("Choose the OptiFine jar you downloaded first");
                    return Task::none();
                }
                let name = if f.name.trim().is_empty() {
                    version.clone()
                } else {
                    f.name.trim().to_string()
                };
                match Instance::create(&self.paths, &name, &version, f.low_end) {
                    Ok(instance) => {
                        let Screen::NewInstance(form) =
                            std::mem::replace(&mut self.screen, Screen::Home)
                        else {
                            unreachable!("checked above")
                        };
                        self.reload_instances();
                        self.selected = Some(name);
                        self.notice = None;
                        return self.start_loader_job(instance, &form.loader);
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }

            Message::ShowEdit(name) => {
                let Some(i) = self.instance(&name) else {
                    return Task::none();
                };
                let current = i.loader.as_ref().map(|l| l.version.clone());
                let form = EditForm {
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
                    loader: LoaderPick {
                        kind: i.loader.as_ref().map(|l| l.kind),
                        versions: None,
                        version: current,
                        optifine_jar: None,
                    },
                };
                self.screen = Screen::EditInstance(form);
                return self.load_loader_versions();
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
                let Some(mut instance) = self.instance(&f.original).cloned() else {
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

                let current = instance
                    .loader
                    .as_ref()
                    .map(|l| (l.kind, l.version.clone()));
                let wanted = f
                    .loader
                    .kind
                    .map(|k| (k, f.loader.version.clone().unwrap_or_default()));
                let loader_changed = match (&current, &wanted) {
                    (None, None) => false,
                    // OptiFine has no version list; picking a new jar is the change.
                    (Some((LoaderKind::OptiFine, _)), Some((LoaderKind::OptiFine, _))) => {
                        f.loader.optifine_jar.is_some()
                    }
                    (Some((ck, cv)), Some((wk, wv))) => ck != wk || cv != wv,
                    _ => true,
                };
                if wanted.is_some_and(|(k, _)| k == LoaderKind::OptiFine)
                    && loader_changed
                    && f.loader.optifine_jar.is_none()
                {
                    self.error("Choose the OptiFine jar you downloaded first");
                    return Task::none();
                }
                if loader_changed {
                    // Back to vanilla first; a new loader (if any) installs on top below.
                    let _ = loader::remove(&self.paths, &mut instance);
                }
                if let Err(e) = instance.save(&self.paths) {
                    self.error(e.to_string());
                    return Task::none();
                }
                let Screen::EditInstance(form) = std::mem::replace(&mut self.screen, Screen::Home)
                else {
                    unreachable!("checked above")
                };
                self.reload_instances();
                self.notice = Some((false, "Saved".into()));
                if loader_changed {
                    return self.start_loader_job(instance, &form.loader);
                }
            }

            Message::PickLoader(kind) => {
                if let Some(pick) = self.picker_mut() {
                    pick.kind = kind;
                    pick.version = None;
                    pick.optifine_jar = None;
                }
                return self.load_loader_versions();
            }
            Message::LoaderVersions(kind, mc, result) => {
                let still_wanted = self.picker_minecraft().as_deref() == Some(mc.as_str());
                if let Some(pick) = self
                    .picker_mut()
                    .filter(|p| still_wanted && p.kind == Some(kind))
                {
                    match result {
                        Ok(versions) => {
                            let keep = pick
                                .version
                                .clone()
                                .filter(|v| versions.iter().any(|x| &x.version == v));
                            pick.version = keep.or_else(|| {
                                loader::recommended(&versions).map(|v| v.version.clone())
                            });
                            pick.versions = Some(Load::Ready(versions));
                        }
                        Err(e) => pick.versions = Some(Load::Failed(e)),
                    }
                }
            }
            Message::PickLoaderVersion(v) => {
                if let Some(pick) = self.picker_mut() {
                    pick.version = Some(v);
                }
            }
            Message::BrowseOptiFine => {
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose the OptiFine jar you downloaded")
                        .add_filter("OptiFine jar", &["jar"])
                        .pick_file(),
                    |file| Message::OptiFinePicked(file.map(|f| f.path().to_path_buf())),
                );
            }
            Message::OptiFinePicked(path) => {
                if let (Some(path), Some(pick)) = (path, self.picker_mut()) {
                    pick.optifine_jar = Some(path);
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

            Message::ShowMods(name) => {
                let Some(instance) = self.instance(&name).cloned() else {
                    return Task::none();
                };
                let has_loader = instance
                    .loader
                    .as_ref()
                    .is_some_and(|l| !l.kind.mod_loaders().is_empty());
                self.screen = Screen::Mods(ModsView {
                    installed: mods::list(&self.paths, &instance).unwrap_or_default(),
                    instance: name,
                    tab: ModsTab::Installed,
                    query: String::new(),
                    results: None,
                    total: 0,
                    installing: HashSet::new(),
                });
                if has_loader {
                    return self.search_mods(0); // preload popular mods for the Browse tab
                }
            }
            Message::ModsTab(tab) => {
                if let Some(view) = self.mods_view() {
                    view.tab = tab;
                }
            }
            Message::ModsQuery(q) => {
                if let Some(view) = self.mods_view() {
                    view.query = q;
                }
            }
            Message::ModsSearch => {
                if let Some(view) = self.mods_view() {
                    view.tab = ModsTab::Browse;
                }
                return self.search_mods(0);
            }
            Message::ModsMore => {
                let offset = match self.mods_view().map(|v| &v.results) {
                    Some(Some(Load::Ready(hits))) => hits.len() as u32,
                    _ => return Task::none(),
                };
                return self.search_mods(offset);
            }
            Message::ModsResults(offset, result) => {
                if let Some(view) = self.mods_view() {
                    match result {
                        Ok(found) => {
                            view.total = found.total_hits;
                            match &mut view.results {
                                Some(Load::Ready(hits)) if offset > 0 => hits.extend(found.hits),
                                _ => view.results = Some(Load::Ready(found.hits)),
                            }
                        }
                        Err(e) => view.results = Some(Load::Failed(e)),
                    }
                }
            }
            Message::InstallMod(project) => {
                let Some(instance) = self.mods_instance() else {
                    return Task::none();
                };
                if let Some(view) = self.mods_view() {
                    view.installing.insert(project.clone());
                }
                let paths = self.paths.clone();
                let id = project.clone();
                return Task::perform(
                    async move {
                        mods::install(
                            &download::client(),
                            &paths,
                            &instance,
                            &id,
                            &download::no_progress(),
                        )
                        .await
                        .map_err(|e| e.to_string())
                    },
                    move |result| Message::ModInstalled(project.clone(), result),
                );
            }
            Message::ModInstalled(project, result) => {
                if let Some(view) = self.mods_view() {
                    view.installing.remove(&project);
                }
                match result {
                    Ok(titles) if titles.is_empty() => {
                        self.notice = Some((false, "Already installed".into()))
                    }
                    Ok(titles) => {
                        self.notice = Some((false, format!("Installed {}", titles.join(", "))))
                    }
                    Err(e) => self.error(e),
                }
                self.refresh_mods();
            }
            Message::ToggleMod(file, enabled) => {
                if let Some(instance) = self.mods_instance()
                    && let Err(e) = mods::set_enabled(&self.paths, &instance, &file, enabled)
                {
                    self.error(e.to_string());
                }
                self.refresh_mods();
            }
            Message::RemoveMod(file) => {
                if let Some(instance) = self.mods_instance()
                    && let Err(e) = mods::remove(&self.paths, &instance, &file)
                {
                    self.error(e.to_string());
                }
                self.refresh_mods();
            }
            Message::AddModFiles => {
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .set_title("Add mods")
                        .add_filter("Mod jar", &["jar"])
                        .pick_files(),
                    |files| {
                        Message::ModFilesPicked(
                            files
                                .unwrap_or_default()
                                .iter()
                                .map(|f| f.path().to_path_buf())
                                .collect(),
                        )
                    },
                );
            }
            Message::ModFilesPicked(files) => {
                for file in files {
                    self.add_mod_file(&file);
                }
            }
            Message::FileDropped(path) => {
                // .mrpack anywhere imports a modpack; .jar on the Mods screen adds a mod.
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("mrpack"))
                {
                    let label = path
                        .file_name()
                        .map_or("modpack".into(), |n| n.to_string_lossy().into_owned());
                    return self.start_pack_job(PackSource::File(path), label);
                }
                self.add_mod_file(&path);
            }

            Message::ShowModpacks => {
                self.screen = Screen::Modpacks(PacksView {
                    query: String::new(),
                    results: None,
                    total: 0,
                });
                return self.search_packs(0);
            }
            Message::PacksQuery(q) => {
                if let Some(view) = self.packs_view() {
                    view.query = q;
                }
            }
            Message::PacksSearch => return self.search_packs(0),
            Message::PacksMore => {
                let offset = match self.packs_view().map(|v| &v.results) {
                    Some(Some(Load::Ready(hits))) => hits.len() as u32,
                    _ => return Task::none(),
                };
                return self.search_packs(offset);
            }
            Message::PacksResults(offset, result) => {
                if let Some(view) = self.packs_view() {
                    match result {
                        Ok(found) => {
                            view.total = found.total_hits;
                            match &mut view.results {
                                Some(Load::Ready(hits)) if offset > 0 => hits.extend(found.hits),
                                _ => view.results = Some(Load::Ready(found.hits)),
                            }
                        }
                        Err(e) => view.results = Some(Load::Failed(e)),
                    }
                }
            }
            Message::InstallPack(project) => {
                let title = match self.packs_view().map(|v| &v.results) {
                    Some(Some(Load::Ready(hits))) => hits
                        .iter()
                        .find(|h| h.project_id == project)
                        .map(|h| h.title.clone()),
                    _ => None,
                };
                return self.start_pack_job(
                    PackSource::Modrinth(project),
                    title.unwrap_or_else(|| "modpack".into()),
                );
            }
            Message::ImportPack => {
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .set_title("Import a Modrinth modpack")
                        .add_filter("Modrinth modpack", &["mrpack"])
                        .pick_file(),
                    |file| Message::PackPicked(file.map(|f| f.path().to_path_buf())),
                );
            }
            Message::PackPicked(path) => {
                if let Some(path) = path {
                    return self.update(Message::FileDropped(path));
                }
            }
            Message::PackJob(key, event) => match event {
                game::JobEvent::Stage { name: stage, total } => {
                    if let Some(job) = self.pack_jobs.get_mut(&key) {
                        job.stage = Some((stage, 0, total));
                    }
                }
                game::JobEvent::Advance => {
                    if let Some((_, done, _)) =
                        self.pack_jobs.get_mut(&key).and_then(|j| j.stage.as_mut())
                    {
                        *done += 1;
                    }
                }
                game::JobEvent::Done(result) => {
                    let label = self
                        .pack_jobs
                        .remove(&key)
                        .map(|j| j.label)
                        .unwrap_or_default();
                    self.reload_instances();
                    match result {
                        Ok(name) => {
                            self.notice = Some((
                                false,
                                format!(
                                    "Installed {label} as '{name}'. Press Play when you're ready!"
                                ),
                            ));
                            self.selected = Some(name);
                        }
                        Err(e) => self.error(format!("Couldn't install {label}: {e}")),
                    }
                    if self.textures.is_none() {
                        return load_textures(self.paths.clone());
                    }
                }
            },

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
}

/// Open a file, folder or URL with the system's default app.
fn open_path(target: &std::ffi::OsStr) {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(target).spawn();
}
