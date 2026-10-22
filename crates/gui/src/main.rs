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
use nethernode_core::settings::Settings;
use nethernode_core::version::{self, ManifestEntry};

use textures::GameTextures;

/// Lines of game output kept per instance; older lines are dropped to bound memory.
const LOG_LIMIT: usize = 2000;

pub fn main() -> iced::Result {
    // GPU renderer unless Settings say otherwise or the machine has 4 GB of RAM or less
    // (the CPU renderer uses ~40 MB instead of ~90 MB). `ICED_BACKEND` still overrides it.
    let settings = Settings::load();
    if std::env::var_os("ICED_BACKEND").is_none()
        && !settings.use_gpu(SystemMemory::current().total_mb)
    {
        // SAFETY: nothing else is running yet; this happens before any thread is spawned.
        unsafe { std::env::set_var("ICED_BACKEND", "tiny-skia") };
    }
    theme::set_cpu_renderer(cpu_renderer());
    iced::application(App::new, App::update, App::view)
        // Bundled fonts: some Windows setups (and Wine) give iced no usable system font,
        // which left text invisible. These also make every OS look the same.
        .font(include_bytes!("../../../assets/fonts/Minecraft-Regular.otf").as_slice())
        .font(include_bytes!("../../../assets/fonts/Minecraft-Bold.otf").as_slice())
        // Inter stays loaded as the fallback for symbols the pixel font lacks (× ✓ ▶ …).
        .font(include_bytes!("../../../assets/fonts/Inter-Regular.ttf").as_slice())
        .font(include_bytes!("../../../assets/fonts/Inter-Bold.ttf").as_slice())
        .font(include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf").as_slice())
        .default_font(ui::UI_FONT)
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

/// Whether iced was told to draw on the CPU (set in `main` before the app starts).
fn cpu_renderer() -> bool {
    std::env::var("ICED_BACKEND").is_ok_and(|b| b.starts_with("tiny-skia"))
}

/// What the Accounts screen shows about an account's custom skin.
pub struct SkinInfo {
    model: nethernode_core::skins::Model,
    face: Option<iced::widget::image::Handle>,
    cape: bool,
}

fn load_skins(paths: &Paths, accounts: &Accounts) -> HashMap<String, SkinInfo> {
    let store = nethernode_core::skins::SkinStore::new(paths);
    accounts
        .accounts
        .iter()
        .filter_map(|a| store.get(&a.name).map(|r| (a.name.clone(), r)))
        .map(|(name, r)| {
            let info = SkinInfo {
                model: r.model,
                face: textures::skin_face(&r.skin),
                cape: r.cape.is_some(),
            };
            (name, info)
        })
        .collect()
}

pub struct App {
    paths: Paths,
    instances: Vec<Instance>,
    selected: Option<String>,
    accounts: Accounts,
    /// Custom skins of this PC's accounts, by account name.
    skins: HashMap<String, SkinInfo>,
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
    settings: Settings,
    /// Shaders need the GPU renderer; the CPU one (low-memory mode) gets a static backdrop.
    gpu: bool,
    hovered: Option<String>,
    /// The version list is being re-fetched from Mojang.
    refreshing_versions: bool,
    /// Modpack installs in progress, by Modrinth project id or file path.
    pack_jobs: HashMap<String, Job>,
    host_share: Option<HostShare>,
    servers: Vec<nethernode_core::server::Server>,
    server_runs: HashMap<String, ServerRun>,
    /// Server downloads in progress, by server name.
    server_jobs: HashMap<String, Job>,
    /// Which server kinds exist for each Minecraft version.
    server_support: HashMap<String, HashMap<nethernode_core::server::ServerKind, Load<bool>>>,
    join_session: Option<JoinSession>,
    /// Which loaders have a release for each Minecraft version (checked when picked).
    loader_support: HashMap<String, HashMap<LoaderKind, Load<bool>>>,
}

/// A launch in progress: downloading, then playing.
pub struct Run {
    stage: Option<(String, u64, u64)>,
    heap: Option<String>,
    stop: Option<oneshot::Sender<()>>,
    /// Port of the world once the player opens it to LAN (read from the game log).
    lan_port: Option<u16>,
}

/// The world this computer is sharing with friends over the internet.
pub struct HostShare {
    /// Instance or server name, shown in the banner.
    instance: String,
    /// Sharing a server (rather than a world opened to LAN in a game).
    is_server: bool,
    /// What to read out: four words, or the full code if those couldn't be published.
    code: Option<String>,
    /// The full code, which always works.
    full_code: Option<String>,
    stop: Option<oneshot::Sender<()>>,
    /// Friends waiting for Allow/Deny.
    requests: Vec<(String, game::JoinReply)>,
    friends: Vec<String>,
}

/// A server running on this PC.
pub struct ServerRun {
    stage: Option<(String, u64, u64)>,
    /// Set once Java is ready and the process started.
    heap_mb: Option<u64>,
    /// The server printed "Done": players can join.
    ready: bool,
    log: VecDeque<String>,
    commands: futures::channel::mpsc::UnboundedSender<String>,
    stop: Option<oneshot::Sender<()>>,
}

pub struct ServersView {
    selected: Option<String>,
    command: String,
    /// Two-click delete.
    confirm_delete: bool,
    settings: Option<ServerSettingsForm>,
}

/// Editable server.properties values (and the whitelist).
pub struct ServerSettingsForm {
    motd: String,
    max_players: String,
    gamemode: String,
    difficulty: String,
    pvp: bool,
    whitelist_on: bool,
    whitelist: String,
    max_memory: String,
}

pub struct NewServerForm {
    name: String,
    version: Option<String>,
    kind: nethernode_core::server::ServerKind,
    eula: bool,
}

/// A friend's world this computer has joined.
pub struct JoinSession {
    status: nethernode_core::share::JoinEvent,
    stop: Option<oneshot::Sender<()>>,
}

/// The LAN port in a game log line: "Started serving on 51234" (server log) or
/// "Local game hosted on port 51234" (chat message).
fn lan_port_from(line: &str) -> Option<u16> {
    ["Started serving on ", "hosted on port "]
        .iter()
        .find_map(|marker| {
            let rest = &line[line.find(marker)? + marker.len()..];
            rest.split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
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
    Settings(SettingsForm),
    JoinFriend { code: String, error: Option<String> },
    Servers(ServersView),
    NewServer(NewServerForm),
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
    /// OptiFine via the one-click open-source alternative (default) rather than a jar.
    optifine_alt: bool,
}

pub struct NewInstanceForm {
    name: String,
    version: Option<String>,
    filter: VersionFilter,
    low_end: bool,
    loader: LoaderPick,
    /// Worlds (folders or .zip files) to import once the instance exists.
    worlds: Vec<PathBuf>,
}

/// Fields of the server settings form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerField {
    Motd,
    MaxPlayers,
    Gamemode,
    Difficulty,
    Pvp,
    WhitelistOn,
    Whitelist,
    MaxMemory,
}

/// Which folder a Settings "Change…" button is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    GameFiles,
    Instances,
    Servers,
}

pub struct SettingsForm {
    data_dir: PathBuf,
    /// `None` keeps instances inside the game files folder.
    instances_dir: Option<PathBuf>,
    /// `None` keeps servers inside the game files folder.
    servers_dir: Option<PathBuf>,
    move_files: bool,
    renderer: nethernode_core::settings::Renderer,
    animated_background: bool,
    skins: bool,
    /// Files are being moved.
    busy: bool,
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
    width: String,
    height: String,
    fullscreen: bool,
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
    EditWindow(String, String),
    EditFullscreen(bool),
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
    /// Open a file picker for an account's skin (`false`) or cape (`true`).
    PickSkin(String, bool),
    SkinPicked(String, bool, Option<PathBuf>),
    SetSkinModel(String, nethernode_core::skins::Model),
    RemoveSkin(String),
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
    LoaderSupport(String, LoaderKind, Result<bool, String>),
    InviteFriends(String),
    Share(game::ShareEvent),
    StopSharing,
    AnswerJoin(usize, bool),
    CopyText(String),
    ShowJoinFriend,
    JoinCodeInput(String),
    StartJoin,
    JoinUpdate(game::JoinUpdate),
    LeaveFriend,
    ShowServers,
    SelectServer(String),
    ShowNewServer,
    NewServerName(String),
    NewServerVersion(String),
    NewServerKind(nethernode_core::server::ServerKind),
    NewServerEula(bool),
    CreateServer,
    ServerJob(String, game::JobEvent),
    StartServer(String),
    StopServer(String),
    ServerUpdate(String, game::ServerUpdate),
    ServerCommandInput(String),
    SendServerCommand,
    AcceptEula(String),
    InviteServer(String),
    PlayOnServer(String),
    DeleteServer(String),
    ServerImportWorld(String, bool),
    ServerSupport(
        String,
        nethernode_core::server::ServerKind,
        Result<bool, String>,
    ),
    ServerWorldPicked(String, Option<PathBuf>),
    ServerWorldImported(String, Result<(), String>),
    EditServerSettings,
    ServerSettingsInput(ServerField, String),
    ServerSettingsToggle(ServerField, bool),
    SaveServerSettings,
    OptiFineAlternative(bool),
    NewAddWorlds(bool),
    NewWorldsPicked(Vec<PathBuf>),
    RemoveNewWorld(usize),
    EditImportWorlds(bool),
    EditWorldsPicked(Vec<PathBuf>),
    WorldsImported(String, Result<Vec<String>, String>),
    ShowSettings,
    SettingsPick(FolderKind),
    SettingsPicked(FolderKind, Option<PathBuf>),
    SettingsResetInstances,
    SettingsResetServers,
    SettingsMoveFiles(bool),
    SettingsRenderer(nethernode_core::settings::Renderer),
    SettingsAnimation(bool),
    SettingsSkins(bool),
    SaveSettings,
    SettingsSaved(Result<Settings, String>),
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
        let settings = Settings::load();
        let paths = settings.paths();
        let interrupted = Instance::clean_interrupted(&paths);
        let instances = Instance::list(&paths).unwrap_or_default();
        let accounts = Accounts::load(&paths).unwrap_or_default();
        let gpu = !cpu_renderer();
        let mut app = Self {
            selected: instances.first().map(|i| i.name.clone()),
            instances,
            skins: load_skins(&paths, &accounts),
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
            settings,
            gpu,
            hovered: None,
            refreshing_versions: true,
            pack_jobs: HashMap::new(),
            loader_support: HashMap::new(),
            host_share: None,
            servers: nethernode_core::server::Server::list(&paths),
            server_runs: HashMap::new(),
            server_jobs: HashMap::new(),
            server_support: HashMap::new(),
            join_session: None,
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
        self.gpu
            && self.settings.animated_background
            && self.runs.values().all(|r| r.heap.is_none())
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

    /// Check which loaders have a release for the picker's Minecraft version (cached).
    fn check_loader_support(&mut self) -> Task<Message> {
        let Some(mc) = self.picker_minecraft() else {
            return Task::none();
        };
        let known = self.loader_support.entry(mc.clone()).or_default();
        let mut tasks = Vec::new();
        for kind in [
            LoaderKind::Fabric,
            LoaderKind::Quilt,
            LoaderKind::Forge,
            LoaderKind::NeoForge,
        ] {
            if known.contains_key(&kind) {
                continue;
            }
            known.insert(kind, Load::Loading);
            let mc = mc.clone();
            tasks.push(Task::perform(
                async move {
                    let found = loader::versions(&download::client(), kind, &mc).await;
                    (
                        mc,
                        kind,
                        found.map(|v| !v.is_empty()).map_err(|e| e.to_string()),
                    )
                },
                |(mc, kind, result)| Message::LoaderSupport(mc, kind, result),
            ));
        }
        Task::batch(tasks)
    }

    /// Check which server kinds exist for `mc` (cached).
    fn check_server_support(&mut self, mc: Option<String>) -> Task<Message> {
        use nethernode_core::server::{self, ServerKind};
        let Some(mc) = mc else { return Task::none() };
        let known = self.server_support.entry(mc.clone()).or_default();
        let mut tasks = Vec::new();
        for kind in [ServerKind::Fabric, ServerKind::Paper] {
            if known.contains_key(&kind) {
                continue;
            }
            known.insert(kind, Load::Loading);
            let mc = mc.clone();
            tasks.push(Task::perform(
                async move {
                    let result = server::available(&download::client(), kind, &mc)
                        .await
                        .map_err(|e| e.to_string());
                    (mc, kind, result)
                },
                |(mc, kind, result)| Message::ServerSupport(mc, kind, result),
            ));
        }
        Task::batch(tasks)
    }

    /// Whether `kind` is known to have no release for `mc` (unknown counts as available).
    pub fn loader_unavailable(&self, mc: &str, kind: LoaderKind) -> bool {
        let lookup = if kind == LoaderKind::OptiFine {
            return false;
        } else {
            kind
        };
        matches!(
            self.loader_support.get(mc).and_then(|m| m.get(&lookup)),
            Some(Load::Ready(false))
        )
    }

    /// Copy worlds (folders or .zip files) into `instance` in the background.
    fn import_worlds(&self, instance: &Instance, sources: Vec<PathBuf>) -> Task<Message> {
        if sources.is_empty() {
            return Task::none();
        }
        let paths = self.paths.clone();
        let instance = instance.clone();
        let name = instance.name.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    sources
                        .iter()
                        .map(|src| {
                            nethernode_core::worlds::import(&paths, &instance, src)
                                .map_err(|e| e.to_string())
                        })
                        .collect::<Result<Vec<String>, String>>()
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            move |result| Message::WorldsImported(name.clone(), result),
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
        let alternative = kind == LoaderKind::OptiFine && pick.optifine_alt;
        let label = if alternative {
            "Installing OptiFine alternative".to_string()
        } else {
            format!("Installing {kind}")
        };
        self.jobs.insert(name.clone(), Job { label, stage: None });
        if alternative {
            let stream = game::job(move |progress| async move {
                let http = download::client();
                let mut instance = instance;
                loader::apply(
                    &http,
                    &paths,
                    &mut instance,
                    LoaderKind::Fabric,
                    None,
                    &progress,
                )
                .await
                .map_err(|e| e.to_string())?;
                let slugs: Vec<&str> = mods::OPTIFINE_ALTERNATIVE
                    .iter()
                    .map(|(slug, _)| *slug)
                    .collect();
                let (installed, skipped) =
                    mods::install_preset(&http, &paths, &instance, &slugs, &progress)
                        .await
                        .map_err(|e| e.to_string())?;
                let mut msg = format!(
                    "OptiFine alternative installed ({} mods on Fabric)",
                    installed.len()
                );
                if !skipped.is_empty() {
                    msg.push_str(&format!(
                        "; not yet available for this version: {}",
                        skipped.join(", ")
                    ));
                }
                Ok(msg)
            });
            return Task::run(stream, move |event| Message::Job(name.clone(), event));
        }
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

    /// Start an instance, optionally straight into a server (quick play).
    fn play(
        &mut self,
        name: String,
        quick_play: Option<nethernode_core::launch::QuickPlay>,
    ) -> Task<Message> {
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
                lan_port: None,
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
        let stream = game::launch(self.paths.clone(), instance, account, quick_play, stop_rx);
        Task::run(stream, move |event| Message::Game(name.clone(), event))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Select(name) => {
                self.selected = Some(name);
                self.confirm_delete = None;
                self.screen = Screen::Home;
            }
            Message::Hover(name) => self.hovered = name,

            Message::Play(name) => return self.play(name, None),

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
                    game::Event::Log(line) => {
                        if let Some(port) = lan_port_from(&line)
                            && let Some(run) = self.runs.get_mut(&name)
                        {
                            run.lan_port = Some(port);
                        }
                        push(line);
                    }
                    game::Event::Finished(ended) => {
                        self.runs.remove(&name);
                        // The shared world is gone with the game.
                        if let Some(share) = self
                            .host_share
                            .as_mut()
                            .filter(|s| !s.is_server && s.instance == name)
                            && let Some(stop) = share.stop.take()
                        {
                            let _ = stop.send(());
                        }
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
                self.settings.animated_background = !self.settings.animated_background;
                if let Err(e) = self.settings.save() {
                    self.error(format!("Couldn't save settings: {e}"));
                }
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
                        optifine_alt: true,
                    },
                    worlds: Vec::new(),
                });
                // Always offer Mojang's current list, not just what was cached at startup.
                self.refreshing_versions = true;
                return Task::batch([
                    load_versions(self.paths.clone()),
                    self.check_loader_support(),
                ]);
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
                // Loader versions (and which loaders exist at all) depend on the version.
                return Task::batch([self.load_loader_versions(), self.check_loader_support()]);
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
                if f.loader.kind == Some(LoaderKind::OptiFine)
                    && !f.loader.optifine_alt
                    && f.loader.optifine_jar.is_none()
                {
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
                        let worlds = self.import_worlds(&instance, form.worlds.clone());
                        return Task::batch([
                            worlds,
                            self.start_loader_job(instance, &form.loader),
                        ]);
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
                    width: i.window_size().0.to_string(),
                    height: i.window_size().1.to_string(),
                    fullscreen: i.fullscreen,
                    loader: LoaderPick {
                        kind: i.loader.as_ref().map(|l| l.kind),
                        versions: None,
                        version: current,
                        optifine_jar: None,
                        // An existing real-OptiFine instance shows the jar route.
                        optifine_alt: !i
                            .loader
                            .as_ref()
                            .is_some_and(|l| l.kind == LoaderKind::OptiFine),
                    },
                };
                self.screen = Screen::EditInstance(form);
                return Task::batch([self.load_loader_versions(), self.check_loader_support()]);
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
            Message::EditWindow(w, h) => {
                if let Screen::EditInstance(f) = &mut self.screen {
                    f.width = w.chars().filter(char::is_ascii_digit).take(5).collect();
                    f.height = h.chars().filter(char::is_ascii_digit).take(5).collect();
                }
            }
            Message::EditFullscreen(on) => {
                if let Screen::EditInstance(f) = &mut self.screen {
                    f.fullscreen = on;
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
                let size = |v: &str| v.parse::<u32>().ok().filter(|n| (320..=16384).contains(n));
                match (size(&f.width), size(&f.height)) {
                    (Some(w), Some(h)) => {
                        // The launcher default isn't stored, so a later default change applies.
                        instance.resolution =
                            ((w, h) != nethernode_core::instance::DEFAULT_WINDOW).then_some((w, h));
                    }
                    _ => {
                        self.error("Window width and height must be numbers between 320 and 16384");
                        return Task::none();
                    }
                }
                instance.fullscreen = f.fullscreen;

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
                        f.loader.optifine_jar.is_some() || f.loader.optifine_alt
                    }
                    (Some((ck, cv)), Some((wk, wv))) => ck != wk || cv != wv,
                    _ => true,
                };
                if wanted.is_some_and(|(k, _)| k == LoaderKind::OptiFine)
                    && loader_changed
                    && f.loader.optifine_jar.is_none()
                    && !f.loader.optifine_alt
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

            Message::InviteFriends(name) => {
                let Some(port) = self.runs.get(&name).and_then(|r| r.lan_port) else {
                    self.error("Open the world to LAN in the game first (Esc → Open to LAN)");
                    return Task::none();
                };
                if self.host_share.is_some() {
                    return Task::none();
                }
                let (stop_tx, stop_rx) = oneshot::channel();
                self.host_share = Some(HostShare {
                    is_server: false,
                    instance: name,
                    code: None,
                    full_code: None,
                    stop: Some(stop_tx),
                    requests: Vec::new(),
                    friends: Vec::new(),
                });
                return Task::run(
                    game::host_share(self.paths.clone(), port, stop_rx),
                    Message::Share,
                );
            }
            Message::Share(event) => {
                let Some(share) = self.host_share.as_mut() else {
                    return Task::none();
                };
                match event {
                    game::ShareEvent::Ready(code, full) => {
                        share.code = Some(code);
                        share.full_code = Some(full);
                    }
                    game::ShareEvent::Request(friend, reply) => {
                        let world = share.instance.clone();
                        share.requests.push((friend, reply));
                        return alert(
                            "A friend wants to join",
                            &format!(
                                "Someone with your code wants to join {world}. Open netherNode to allow or deny."
                            ),
                            true,
                        );
                    }
                    game::ShareEvent::Joined(friend) => {
                        self.notice = Some((false, format!("{friend} joined your world")));
                        share.friends.push(friend.clone());
                        return alert(
                            "Friend joined",
                            &format!("{friend} is connected to {}", share.instance),
                            false,
                        );
                    }
                    game::ShareEvent::Left(friend) => share.friends.retain(|f| *f != friend),
                    game::ShareEvent::Ended(result) => {
                        self.host_share = None;
                        if let Err(e) = result {
                            self.error(format!("Sharing stopped: {e}"));
                        }
                    }
                }
            }
            Message::StopSharing => {
                if let Some(stop) = self.host_share.as_mut().and_then(|s| s.stop.take()) {
                    let _ = stop.send(());
                }
            }
            Message::AnswerJoin(index, allow) => {
                if let Some(share) = self.host_share.as_mut()
                    && index < share.requests.len()
                {
                    let (_, reply) = share.requests.remove(index);
                    if let Some(reply) = reply.lock().unwrap().take() {
                        let _ = reply.send(allow);
                    }
                }
            }
            Message::CopyText(text) => return iced::clipboard::write(text),
            Message::ShowJoinFriend => {
                self.screen = Screen::JoinFriend {
                    code: String::new(),
                    error: None,
                };
            }
            Message::JoinCodeInput(value) => {
                if let Screen::JoinFriend { code, error } = &mut self.screen {
                    *code = value;
                    *error = None;
                }
            }
            Message::StartJoin => {
                let Screen::JoinFriend { code, error } = &mut self.screen else {
                    return Task::none();
                };
                if !nethernode_core::invite::looks_valid(code) {
                    *error = Some("That isn't a netherNode code: it should be four words, like maple-orbit-tunnel-frog".into());
                    return Task::none();
                }
                let parsed = code.trim().to_string();
                if self.join_session.is_some() {
                    *error = Some("You're already in a friend's world; leave it first".into());
                    return Task::none();
                }
                let (stop_tx, stop_rx) = oneshot::channel();
                self.join_session = Some(JoinSession {
                    status: nethernode_core::share::JoinEvent::Connecting,
                    stop: Some(stop_tx),
                });
                self.screen = Screen::Home;
                return Task::run(
                    game::join_share(self.paths.clone(), parsed, stop_rx),
                    Message::JoinUpdate,
                );
            }
            Message::JoinUpdate(update) => match update {
                game::JoinUpdate::Status(status) => {
                    if let Some(session) = self.join_session.as_mut() {
                        session.status = status;
                    }
                }
                game::JoinUpdate::Ended(result) => {
                    let was_in = matches!(
                        self.join_session.as_ref().map(|s| &s.status),
                        Some(nethernode_core::share::JoinEvent::Ready { .. })
                    );
                    self.join_session = None;
                    match result {
                        Ok(()) => self.notice = Some((false, "Left your friend's world".into())),
                        Err(e) if was_in => self.error(format!("Disconnected: {e}")),
                        Err(e) => self.error(format!("Couldn't join: {e}")),
                    }
                }
            },
            Message::LeaveFriend => {
                if let Some(stop) = self.join_session.as_mut().and_then(|s| s.stop.take()) {
                    let _ = stop.send(());
                }
            }

            Message::ShowServers => {
                self.servers = nethernode_core::server::Server::list(&self.paths);
                let selected = match &self.screen {
                    Screen::Servers(v) => v.selected.clone(),
                    _ => None,
                }
                .filter(|n| self.servers.iter().any(|s| &s.name == n))
                .or_else(|| self.servers.first().map(|s| s.name.clone()));
                self.screen = Screen::Servers(ServersView {
                    selected,
                    command: String::new(),
                    confirm_delete: false,
                    settings: None,
                });
            }
            Message::SelectServer(name) => {
                if let Screen::Servers(v) = &mut self.screen {
                    v.selected = Some(name);
                    v.confirm_delete = false;
                    v.settings = None;
                }
            }
            Message::ShowNewServer => {
                let version = self
                    .version_choices(VersionFilter::Releases)
                    .first()
                    .map(|c| c.id.clone());
                self.screen = Screen::NewServer(NewServerForm {
                    name: String::new(),
                    version: version.clone(),
                    kind: nethernode_core::server::ServerKind::Vanilla,
                    eula: false,
                });
                return self.check_server_support(version);
            }
            Message::NewServerName(name) => {
                if let Screen::NewServer(f) = &mut self.screen {
                    f.name = name;
                }
            }
            Message::NewServerVersion(version) => {
                if let Screen::NewServer(f) = &mut self.screen {
                    f.version = Some(version.clone());
                }
                return self.check_server_support(Some(version));
            }
            Message::NewServerKind(kind) => {
                if let Screen::NewServer(f) = &mut self.screen {
                    f.kind = kind;
                }
            }
            Message::NewServerEula(on) => {
                if let Screen::NewServer(f) = &mut self.screen {
                    f.eula = on;
                }
            }
            Message::ServerSupport(mc, kind, result) => {
                let state = match result {
                    Ok(available) => Load::Ready(available),
                    Err(e) => Load::Failed(e),
                };
                self.server_support
                    .entry(mc)
                    .or_default()
                    .insert(kind, state);
            }
            Message::CreateServer => {
                use nethernode_core::server::{self, Server};
                let Screen::NewServer(f) = &self.screen else {
                    return Task::none();
                };
                let Some(version) = f.version.clone() else {
                    self.error("Pick a Minecraft version first");
                    return Task::none();
                };
                if !f.eula {
                    self.error(
                        "Tick the box to accept the Minecraft EULA; servers can't start without it",
                    );
                    return Task::none();
                }
                let name = if f.name.trim().is_empty() {
                    format!("{} {version}", f.kind.name())
                } else {
                    f.name.trim().to_string()
                };
                let mut created = match Server::create(&self.paths, &name, &version, f.kind) {
                    Ok(created) => created,
                    Err(e) => {
                        self.error(e.to_string());
                        return Task::none();
                    }
                };
                if let Err(e) = server::accept_eula(&self.paths, &created) {
                    self.error(e.to_string());
                    return Task::none();
                }
                self.servers = Server::list(&self.paths);
                self.server_jobs.insert(
                    name.clone(),
                    Job {
                        label: "Downloading the server".into(),
                        stage: None,
                    },
                );
                self.screen = Screen::Servers(ServersView {
                    selected: Some(name.clone()),
                    command: String::new(),
                    confirm_delete: false,
                    settings: None,
                });
                let paths = self.paths.clone();
                let stream = game::job(move |progress| async move {
                    let result =
                        server::install(&download::client(), &paths, &mut created, &progress).await;
                    if result.is_err() {
                        let _ = Server::delete(&paths, &created.name);
                    }
                    result
                        .map(|()| "ready to start".to_string())
                        .map_err(|e| e.to_string())
                });
                return Task::run(stream, move |event| Message::ServerJob(name.clone(), event));
            }
            Message::ServerJob(name, event) => match event {
                game::JobEvent::Stage { name: stage, total } => {
                    if let Some(job) = self.server_jobs.get_mut(&name) {
                        job.stage = Some((stage, 0, total));
                    }
                }
                game::JobEvent::Advance => {
                    if let Some((_, done, _)) = self
                        .server_jobs
                        .get_mut(&name)
                        .and_then(|j| j.stage.as_mut())
                    {
                        *done += 1;
                    }
                }
                game::JobEvent::Done(result) => {
                    self.server_jobs.remove(&name);
                    self.servers = nethernode_core::server::Server::list(&self.paths);
                    match result {
                        Ok(msg) => self.notice = Some((false, format!("Server {name}: {msg}"))),
                        Err(e) => self.error(format!("Couldn't set up {name}: {e}")),
                    }
                }
            },
            Message::StartServer(name) => {
                if self.server_runs.contains_key(&name) {
                    return Task::none();
                }
                let Some(server) = self.servers.iter().find(|s| s.name == name).cloned() else {
                    return Task::none();
                };
                let (commands_tx, commands_rx) = futures::channel::mpsc::unbounded();
                let (stop_tx, stop_rx) = oneshot::channel();
                self.server_runs.insert(
                    name.clone(),
                    ServerRun {
                        stage: None,
                        heap_mb: None,
                        ready: false,
                        log: VecDeque::from([format!("Starting {name}…")]),
                        commands: commands_tx,
                        stop: Some(stop_tx),
                    },
                );
                let stream = game::run_server(self.paths.clone(), server, commands_rx, stop_rx);
                return Task::run(stream, move |update| {
                    Message::ServerUpdate(name.clone(), update)
                });
            }
            Message::StopServer(name) => {
                if let Some(stop) = self.server_runs.get_mut(&name).and_then(|r| r.stop.take()) {
                    let _ = stop.send(());
                }
            }
            Message::ServerUpdate(name, update) => {
                let port = self
                    .servers
                    .iter()
                    .find(|s| s.name == name)
                    .map_or(0, |s| s.port);
                let Some(run) = self.server_runs.get_mut(&name) else {
                    return Task::none();
                };
                match update {
                    game::ServerUpdate::Stage { name: stage, total } => {
                        run.stage = Some((stage, 0, total))
                    }
                    game::ServerUpdate::Advance => {
                        if let Some((_, done, _)) = run.stage.as_mut() {
                            *done += 1;
                        }
                    }
                    game::ServerUpdate::Started { heap_mb } => {
                        run.stage = None;
                        run.heap_mb = Some(heap_mb);
                    }
                    game::ServerUpdate::Line(line) => {
                        // "Done (12.3s)! For help, type "help"" means players can join.
                        let became_ready = !run.ready && line.contains("Done (");
                        if run.log.len() >= LOG_LIMIT {
                            run.log.pop_front();
                        }
                        run.log.push_back(line);
                        if became_ready {
                            run.ready = true;
                            return alert(
                                "Server ready",
                                &format!("{name} is running (port {port})"),
                                false,
                            );
                        }
                    }
                    game::ServerUpdate::Ended(result) => {
                        self.server_runs.remove(&name);
                        if let Some(share) = self
                            .host_share
                            .as_mut()
                            .filter(|s| s.is_server && s.instance == name)
                            && let Some(stop) = share.stop.take()
                        {
                            let _ = stop.send(());
                        }
                        match result {
                            Ok(msg) => self.notice = Some((false, format!("{name}: {msg}"))),
                            Err(e) => self.error(format!("{name}: {e}")),
                        }
                    }
                }
            }
            Message::ServerCommandInput(value) => {
                if let Screen::Servers(v) = &mut self.screen {
                    v.command = value;
                }
            }
            Message::SendServerCommand => {
                let Screen::Servers(v) = &mut self.screen else {
                    return Task::none();
                };
                let command = std::mem::take(&mut v.command);
                if let (Some(run), false) = (
                    v.selected
                        .as_ref()
                        .and_then(|n| self.server_runs.get_mut(n)),
                    command.trim().is_empty(),
                ) {
                    run.log.push_back(format!("> {command}"));
                    let _ = run.commands.unbounded_send(command);
                }
            }
            Message::AcceptEula(name) => {
                if let Some(server) = self.servers.iter().find(|s| s.name == name)
                    && let Err(e) = nethernode_core::server::accept_eula(&self.paths, server)
                {
                    self.error(e.to_string());
                }
            }
            Message::InviteServer(name) => {
                let Some(server) = self.servers.iter().find(|s| s.name == name) else {
                    return Task::none();
                };
                if !self.server_runs.get(&name).is_some_and(|r| r.ready) {
                    self.error("Start the server first");
                    return Task::none();
                }
                if self.host_share.is_some() {
                    self.error("Already sharing something; stop that first");
                    return Task::none();
                }
                let port = server.port;
                let (stop_tx, stop_rx) = oneshot::channel();
                self.host_share = Some(HostShare {
                    instance: name,
                    is_server: true,
                    code: None,
                    full_code: None,
                    stop: Some(stop_tx),
                    requests: Vec::new(),
                    friends: Vec::new(),
                });
                return Task::run(
                    game::host_share(self.paths.clone(), port, stop_rx),
                    Message::Share,
                );
            }
            Message::PlayOnServer(name) => {
                let Some(server) = self.servers.iter().find(|s| s.name == name).cloned() else {
                    return Task::none();
                };
                // The selected instance if it's the right version, else any that is.
                let instance = self
                    .selected
                    .as_ref()
                    .and_then(|n| self.instance(n))
                    .filter(|i| i.minecraft_version() == server.minecraft)
                    .or_else(|| {
                        self.instances
                            .iter()
                            .find(|i| i.minecraft_version() == server.minecraft)
                    })
                    .map(|i| i.name.clone());
                let Some(instance) = instance else {
                    self.error(format!(
                        "Create an instance for Minecraft {} to play on this server",
                        server.minecraft
                    ));
                    return Task::none();
                };
                let target = nethernode_core::launch::QuickPlay::Multiplayer(format!(
                    "localhost:{}",
                    server.port
                ));
                return self.play(instance, Some(target));
            }
            Message::DeleteServer(name) => {
                if self.server_runs.contains_key(&name) {
                    self.error("Stop the server before deleting it");
                    return Task::none();
                }
                let Screen::Servers(v) = &mut self.screen else {
                    return Task::none();
                };
                if !v.confirm_delete {
                    v.confirm_delete = true;
                    return Task::none();
                }
                v.confirm_delete = false;
                v.selected = None;
                match nethernode_core::server::Server::delete(&self.paths, &name) {
                    Ok(()) => self.notice = Some((false, format!("Deleted server {name}"))),
                    Err(e) => self.error(e.to_string()),
                }
                return Task::done(Message::ShowServers);
            }
            Message::ServerImportWorld(name, zip) => {
                if self.server_runs.contains_key(&name) {
                    self.error("Stop the server before replacing its world");
                    return Task::none();
                }
                let dialog = rfd::AsyncFileDialog::new();
                let pick = move |file: Option<rfd::FileHandle>| {
                    Message::ServerWorldPicked(name.clone(), file.map(|f| f.path().to_path_buf()))
                };
                return if zip {
                    Task::perform(
                        dialog
                            .set_title("Choose a world .zip")
                            .add_filter("World backup", &["zip"])
                            .pick_file(),
                        pick,
                    )
                } else {
                    Task::perform(
                        dialog
                            .set_title("Choose a world folder (with level.dat inside)")
                            .pick_folder(),
                        pick,
                    )
                };
            }
            Message::ServerWorldPicked(name, Some(source)) => {
                let Some(server) = self.servers.iter().find(|s| s.name == name).cloned() else {
                    return Task::none();
                };
                let paths = self.paths.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            nethernode_core::server::import_world(&paths, &server, &source)
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    move |result| Message::ServerWorldImported(name.clone(), result),
                );
            }
            Message::ServerWorldPicked(_, None) => {}
            Message::ServerWorldImported(name, result) => match result {
                Ok(()) => {
                    self.notice = Some((
                        false,
                        format!(
                            "{name} will use the imported world (the old one was kept as a backup)"
                        ),
                    ))
                }
                Err(e) => self.error(format!("Couldn't import the world: {e}")),
            },
            Message::EditServerSettings => {
                let Screen::Servers(v) = &mut self.screen else {
                    return Task::none();
                };
                let Some(server) = v
                    .selected
                    .as_ref()
                    .and_then(|n| self.servers.iter().find(|s| &s.name == n))
                else {
                    return Task::none();
                };
                let props = nethernode_core::server::properties(&server.dir(&self.paths));
                let get = |k: &str, default: &str| {
                    props.get(k).cloned().unwrap_or_else(|| default.to_string())
                };
                v.settings = Some(ServerSettingsForm {
                    motd: get("motd", ""),
                    max_players: get("max-players", "10"),
                    gamemode: get("gamemode", "survival"),
                    difficulty: get("difficulty", "normal"),
                    pvp: get("pvp", "true") == "true",
                    whitelist_on: get("white-list", "false") == "true",
                    whitelist: nethernode_core::server::whitelist(&self.paths, server).join(", "),
                    max_memory: server
                        .max_memory_mb
                        .map(|m| m.to_string())
                        .unwrap_or_default(),
                });
            }
            Message::ServerSettingsInput(field, value) => {
                if let Screen::Servers(ServersView {
                    settings: Some(f), ..
                }) = &mut self.screen
                {
                    match field {
                        ServerField::Motd => f.motd = value,
                        ServerField::MaxPlayers => {
                            f.max_players =
                                value.chars().filter(char::is_ascii_digit).take(4).collect()
                        }
                        ServerField::Gamemode => f.gamemode = value,
                        ServerField::Difficulty => f.difficulty = value,
                        ServerField::Whitelist => f.whitelist = value,
                        ServerField::MaxMemory => {
                            f.max_memory =
                                value.chars().filter(char::is_ascii_digit).take(6).collect()
                        }
                        ServerField::Pvp | ServerField::WhitelistOn => {}
                    }
                }
            }
            Message::ServerSettingsToggle(field, on) => {
                if let Screen::Servers(ServersView {
                    settings: Some(f), ..
                }) = &mut self.screen
                {
                    match field {
                        ServerField::Pvp => f.pvp = on,
                        ServerField::WhitelistOn => f.whitelist_on = on,
                        _ => {}
                    }
                }
            }
            Message::SaveServerSettings => {
                use nethernode_core::server;
                let Screen::Servers(v) = &mut self.screen else {
                    return Task::none();
                };
                let (Some(name), Some(f)) = (v.selected.clone(), v.settings.as_ref()) else {
                    return Task::none();
                };
                let Some(mut srv) = self.servers.iter().find(|s| s.name == name).cloned() else {
                    return Task::none();
                };
                let names: Vec<String> = f
                    .whitelist
                    .split([',', ' ', '\n'])
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(String::from)
                    .collect();
                srv.max_memory_mb = f.max_memory.parse::<u32>().ok().filter(|m| *m >= 512);
                let dir = srv.dir(&self.paths);
                let pvp = if f.pvp { "true" } else { "false" };
                let result = server::set_properties(
                    &dir,
                    &[
                        ("motd", f.motd.as_str()),
                        (
                            "max-players",
                            if f.max_players.is_empty() {
                                "10"
                            } else {
                                f.max_players.as_str()
                            },
                        ),
                        ("gamemode", f.gamemode.as_str()),
                        ("difficulty", f.difficulty.as_str()),
                        ("pvp", pvp),
                    ],
                )
                .and_then(|()| server::set_whitelist(&self.paths, &srv, f.whitelist_on, &names))
                .and_then(|()| srv.save(&self.paths));
                match result {
                    Ok(()) => {
                        v.settings = None;
                        self.servers = server::Server::list(&self.paths);
                        let running = self.server_runs.contains_key(&name);
                        self.notice = Some((
                            false,
                            if running {
                                "Saved. Restart the server to apply.".into()
                            } else {
                                "Server settings saved".into()
                            },
                        ));
                    }
                    Err(e) => self.error(e.to_string()),
                }
            }

            Message::LoaderSupport(mc, kind, result) => {
                // A failed check (e.g. offline) leaves the loader selectable.
                let state = match result {
                    Ok(available) => Load::Ready(available),
                    Err(e) => Load::Failed(e),
                };
                self.loader_support
                    .entry(mc)
                    .or_default()
                    .insert(kind, state);
            }
            Message::OptiFineAlternative(on) => {
                if let Some(pick) = self.picker_mut() {
                    pick.optifine_alt = on;
                }
            }

            Message::NewAddWorlds(zip) => return pick_worlds(zip, Message::NewWorldsPicked),
            Message::NewWorldsPicked(sources) => {
                if let Screen::NewInstance(f) = &mut self.screen {
                    for source in sources {
                        if !f.worlds.contains(&source) {
                            f.worlds.push(source);
                        }
                    }
                }
            }
            Message::RemoveNewWorld(index) => {
                if let Screen::NewInstance(f) = &mut self.screen
                    && index < f.worlds.len()
                {
                    f.worlds.remove(index);
                }
            }
            Message::EditImportWorlds(zip) => return pick_worlds(zip, Message::EditWorldsPicked),
            Message::EditWorldsPicked(sources) => {
                let Screen::EditInstance(f) = &self.screen else {
                    return Task::none();
                };
                let Some(instance) = self.instance(&f.original).cloned() else {
                    return Task::none();
                };
                return self.import_worlds(&instance, sources);
            }
            Message::WorldsImported(name, result) => match result {
                Ok(worlds) if worlds.is_empty() => {}
                Ok(worlds) => {
                    self.notice = Some((
                        false,
                        format!("Imported into {name}: {}", worlds.join(", ")),
                    ))
                }
                Err(e) => self.error(format!("Couldn't import the world into {name}: {e}")),
            },

            Message::ShowSettings => {
                let default_instances = self.paths.root.join("instances");
                self.screen = Screen::Settings(SettingsForm {
                    data_dir: self.paths.root.clone(),
                    instances_dir: Some(self.paths.instances()).filter(|d| *d != default_instances),
                    servers_dir: Some(self.paths.servers())
                        .filter(|d| *d != self.paths.root.join("servers")),
                    move_files: true,
                    renderer: self.settings.renderer,
                    animated_background: self.settings.animated_background,
                    skins: self.settings.skins,
                    busy: false,
                });
            }
            Message::SettingsPick(which) => {
                let title = match which {
                    FolderKind::GameFiles => "Folder for game files (versions, libraries, Java)",
                    FolderKind::Instances => "Folder for instances and worlds",
                    FolderKind::Servers => "Folder for servers and their worlds",
                };
                return Task::perform(
                    rfd::AsyncFileDialog::new().set_title(title).pick_folder(),
                    move |folder| {
                        Message::SettingsPicked(which, folder.map(|f| f.path().to_path_buf()))
                    },
                );
            }
            Message::SettingsPicked(which, Some(folder)) => {
                if let Screen::Settings(f) = &mut self.screen {
                    match which {
                        FolderKind::GameFiles => f.data_dir = folder,
                        FolderKind::Instances => f.instances_dir = Some(folder),
                        FolderKind::Servers => f.servers_dir = Some(folder),
                    }
                }
            }
            Message::SettingsPicked(_, None) => {}
            Message::SettingsResetInstances => {
                if let Screen::Settings(f) = &mut self.screen {
                    f.instances_dir = None;
                }
            }
            Message::SettingsResetServers => {
                if let Screen::Settings(f) = &mut self.screen {
                    f.servers_dir = None;
                }
            }
            Message::SettingsMoveFiles(on) => {
                if let Screen::Settings(f) = &mut self.screen {
                    f.move_files = on;
                }
            }
            Message::SettingsRenderer(renderer) => {
                if let Screen::Settings(f) = &mut self.screen {
                    f.renderer = renderer;
                }
            }
            Message::SettingsAnimation(on) => {
                if let Screen::Settings(f) = &mut self.screen {
                    f.animated_background = on;
                }
            }
            Message::SettingsSkins(on) => {
                if let Screen::Settings(f) = &mut self.screen {
                    f.skins = on;
                }
            }
            Message::SaveSettings => {
                let busy =
                    !self.runs.is_empty() || !self.jobs.is_empty() || !self.pack_jobs.is_empty();
                let Screen::Settings(f) = &mut self.screen else {
                    return Task::none();
                };
                let new = Settings {
                    // Stored only when it differs from the platform default.
                    data_dir: Some(f.data_dir.clone())
                        .filter(|d| *d != nethernode_core::paths::Paths::default_root()),
                    instances_dir: f.instances_dir.clone(),
                    servers_dir: f.servers_dir.clone(),
                    renderer: f.renderer,
                    animated_background: f.animated_background,
                    skins: f.skins,
                };
                let old = self.paths.clone();
                let target = new.paths();
                let folders_changed = target.root != old.root
                    || target.instances() != old.instances()
                    || target.servers() != old.servers();
                let busy = busy || !self.server_runs.is_empty();
                if folders_changed && busy {
                    self.error("Stop running games and finish installs before moving folders");
                    return Task::none();
                }
                if !(folders_changed && f.move_files) {
                    return Task::done(Message::SettingsSaved(Ok(new)));
                }
                f.busy = true;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            use nethernode_core::settings::move_dir;
                            // Instances first (they may live inside the game files folder).
                            if target.instances() != old.instances() {
                                move_dir(&old.instances(), &target.instances())
                                    .map_err(|e| e.to_string())?;
                            }
                            if target.servers() != old.servers() {
                                move_dir(&old.servers(), &target.servers())
                                    .map_err(|e| e.to_string())?;
                            }
                            if target.root != old.root {
                                move_dir(&old.root, &target.root).map_err(|e| e.to_string())?;
                            }
                            Ok(new)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    Message::SettingsSaved,
                );
            }
            Message::SettingsSaved(result) => match result {
                Ok(new) => {
                    let renderer_changed = new.renderer != self.settings.renderer;
                    if let Err(e) = new.save() {
                        self.error(format!("Couldn't save settings: {e}"));
                        return Task::none();
                    }
                    self.paths = new.paths();
                    self.settings = new;
                    self.accounts = Accounts::load(&self.paths).unwrap_or_default();
                    self.reload_instances();
                    self.screen = Screen::Home;
                    self.notice = Some((
                        false,
                        if renderer_changed {
                            "Settings saved. Restart netherNode to switch the renderer.".into()
                        } else {
                            "Settings saved".into()
                        },
                    ));
                    return Task::batch([
                        load_versions(self.paths.clone()),
                        load_textures(self.paths.clone()),
                    ]);
                }
                Err(e) => {
                    if let Screen::Settings(f) = &mut self.screen {
                        f.busy = false;
                    }
                    self.error(format!("Couldn't move the folders: {e}"));
                }
            },

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
            Message::PickSkin(name, cape) => {
                let title = if cape {
                    "Choose a cape (64×32 PNG)"
                } else {
                    "Choose a skin (64×64 PNG)"
                };
                return Task::perform(
                    rfd::AsyncFileDialog::new()
                        .set_title(title)
                        .add_filter("PNG image", &["png"])
                        .pick_file(),
                    move |file| {
                        Message::SkinPicked(
                            name.clone(),
                            cape,
                            file.map(|f| f.path().to_path_buf()),
                        )
                    },
                );
            }
            Message::SkinPicked(name, cape, Some(path)) => {
                let store = nethernode_core::skins::SkinStore::new(&self.paths);
                let model = self.skins.get(&name).map(|s| s.model).unwrap_or_default();
                let result = std::fs::read(&path).map_err(Into::into).and_then(|png| {
                    if cape {
                        store.set_own_cape(&name, Some(&png))
                    } else {
                        store.set_own_skin(&name, &png, model)
                    }
                });
                match result {
                    Ok(()) => self.notice = None,
                    Err(e) => self.error(e.to_string()),
                }
                self.skins = load_skins(&self.paths, &self.accounts);
            }
            Message::SkinPicked(_, _, None) => {}
            Message::SetSkinModel(name, model) => {
                let store = nethernode_core::skins::SkinStore::new(&self.paths);
                if let Err(e) = store.set_own_model(&name, model) {
                    self.error(e.to_string());
                }
                self.skins = load_skins(&self.paths, &self.accounts);
            }
            Message::RemoveSkin(name) => {
                let store = nethernode_core::skins::SkinStore::new(&self.paths);
                if let Err(e) = store.remove_own(&name) {
                    self.error(e.to_string());
                }
                self.skins = load_skins(&self.paths, &self.accounts);
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

/// Desktop notification, plus a taskbar flash / dock bounce when `urgent`, for things the
/// player should see while they're in the game.
fn alert(title: &str, body: &str, urgent: bool) -> Task<Message> {
    let (title, body) = (title.to_string(), body.to_string());
    // Showing a notification can block briefly (D-Bus, WinRT), so keep it off the UI thread.
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname("netherNode")
            .summary(&title)
            .body(&body)
            .show();
    });
    if urgent {
        window::latest().and_then(|id| {
            window::request_user_attention(id, Some(window::UserAttention::Critical))
        })
    } else {
        Task::none()
    }
}

/// Ask for world folders (each containing level.dat) or world .zip files.
fn pick_worlds(zip: bool, done: fn(Vec<PathBuf>) -> Message) -> Task<Message> {
    let to_paths = move |files: Option<Vec<rfd::FileHandle>>| {
        done(
            files
                .unwrap_or_default()
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect(),
        )
    };
    let dialog = rfd::AsyncFileDialog::new();
    if zip {
        Task::perform(
            dialog
                .set_title("Choose world .zip files")
                .add_filter("World backup", &["zip"])
                .pick_files(),
            to_paths,
        )
    } else {
        Task::perform(
            dialog
                .set_title("Choose world folders (the ones with level.dat inside)")
                .pick_folders(),
            to_paths,
        )
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

#[cfg(test)]
mod tests {
    use super::lan_port_from;

    #[test]
    fn finds_lan_port_in_game_log() {
        assert_eq!(
            lan_port_from("[12:00:01] [Server thread/INFO]: Started serving on 51234"),
            Some(51234)
        );
        assert_eq!(
            lan_port_from("[Render thread/INFO]: [CHAT] Local game hosted on port 40000"),
            Some(40000)
        );
        assert_eq!(
            lan_port_from("[Server thread/INFO]: Starting integrated minecraft server"),
            None
        );
    }
}
