use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use nethernode_core::account::Accounts;
use nethernode_core::download::{self, Progress, ProgressFn};
use nethernode_core::instance::Instance;
use nethernode_core::launch::{self, QuickPlay};
use nethernode_core::loader::{self, LoaderKind};
use nethernode_core::memory::SystemMemory;
use nethernode_core::paths::Paths;
use nethernode_core::version;
use nethernode_core::{modpack, mods, share};

#[derive(Parser)]
#[command(
    name = "nethernode",
    version,
    about = "just another minecraft launcher"
)]
struct Cli {
    /// Data directory (default: $NETHERNODE_HOME or the platform data dir)
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List available Minecraft versions
    Versions {
        /// Include snapshots
        #[arg(long)]
        snapshots: bool,
        /// Include old_alpha / old_beta
        #[arg(long)]
        old: bool,
        /// Show at most this many
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Manage offline accounts
    #[command(subcommand)]
    Account(AccountCmd),
    /// Manage instances
    #[command(subcommand)]
    Instance(InstanceCmd),
    /// Mod loaders: Fabric, Quilt, Forge, NeoForge, OptiFine
    #[command(subcommand)]
    Loader(LoaderCmd),
    /// Mods for an instance (from Modrinth or local files)
    #[command(subcommand)]
    Mods(ModsCmd),
    /// Modrinth modpacks: import .mrpack files or install from Modrinth
    #[command(subcommand)]
    Modpack(ModpackCmd),
    /// Play a LAN world with friends over the internet (no port forwarding)
    #[command(subcommand)]
    Share(ShareCmd),
    /// Permanent friends: see who's online and join what they share
    #[command(subcommand)]
    Friends(FriendsCmd),
    /// Run the local skin service until Ctrl+C (for troubleshooting skins)
    #[command(hide = true)]
    SkinService,
    /// Minecraft servers hosted on this PC
    #[command(subcommand)]
    Server(ServerCmd),
    /// Download (if needed) and start an instance
    Launch {
        instance: String,
        /// Account name (default: the default account)
        #[arg(long, short)]
        account: Option<String>,
        /// Join this singleplayer world on start
        #[arg(long, conflicts_with = "server")]
        world: Option<String>,
        /// Join this server (host[:port]) on start
        #[arg(long)]
        server: Option<String>,
        /// Prepare everything and print the command instead of running it
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum AccountCmd {
    /// Add an offline account
    Add { name: String },
    /// List accounts
    List,
    /// Remove an account
    Remove { name: String },
    /// Set the default account
    Default { name: String },
    /// Give an account a custom skin (a 64x64 PNG), seen in game by you and your friends
    Skin {
        name: String,
        png: PathBuf,
        /// Slim (Alex-style) arms
        #[arg(long)]
        slim: bool,
        /// A cape (64x32 PNG)
        #[arg(long)]
        cape: Option<PathBuf>,
    },
    /// Go back to the default skin
    RemoveSkin { name: String },
}

#[derive(Subcommand)]
enum InstanceCmd {
    /// Create an instance. VERSION may be `latest`, `snapshot` or an id like 1.21.1
    Create {
        name: String,
        version: String,
        /// Max heap in MB (default: automatic, based on this machine's RAM)
        #[arg(long)]
        max_memory: Option<u32>,
        /// Initial heap in MB (default: 256)
        #[arg(long)]
        min_memory: Option<u32>,
        /// Lighter graphics defaults in options.txt (default: on for machines with 8 GB RAM or less)
        #[arg(long, overrides_with = "no_low_end")]
        low_end: bool,
        #[arg(long, hide = true)]
        no_low_end: bool,
        /// Use this java executable instead of auto-selecting one
        #[arg(long)]
        java: Option<PathBuf>,
        /// Also install a mod loader: fabric, quilt, forge or neoforge
        #[arg(long)]
        loader: Option<String>,
        /// Game window size, e.g. 1920x1080 (default 1280x720)
        #[arg(long, value_parser = parse_resolution)]
        resolution: Option<(u32, u32)>,
        /// Start the game fullscreen
        #[arg(long)]
        fullscreen: bool,
    },
    /// List instances
    List,
    /// Delete an instance and all its worlds
    Delete {
        name: String,
        /// Required, since this removes saves
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum LoaderCmd {
    /// List a loader's versions for a Minecraft version
    Versions { loader: String, minecraft: String },
    /// Put a loader on an instance (recommended version unless --version is given)
    Install {
        instance: String,
        loader: String,
        #[arg(long)]
        version: Option<String>,
        /// OptiFine only: the jar downloaded from https://optifine.net/downloads
        #[arg(long)]
        jar: Option<PathBuf>,
    },
    /// Remove the loader, back to vanilla
    Remove { instance: String },
}

#[derive(Subcommand)]
enum ModsCmd {
    /// List installed mods
    List {
        instance: String,
    },
    /// Search Modrinth for mods compatible with the instance
    Search {
        instance: String,
        #[arg(default_value = "")]
        query: String,
    },
    /// Install a Modrinth mod (slug or id) plus its required dependencies
    Add {
        instance: String,
        project: String,
    },
    /// List required mods that are missing; with --fix, install them from Modrinth
    Check {
        instance: String,
        #[arg(long)]
        fix: bool,
    },
    /// Copy a mod jar into the instance
    AddFile {
        instance: String,
        file: PathBuf,
    },
    /// Install the open-source OptiFine alternative (Sodium, Iris, Continuity, ...) on a Fabric/Quilt instance
    OptifineAlternative {
        instance: String,
    },
    Enable {
        instance: String,
        file: String,
    },
    Disable {
        instance: String,
        file: String,
    },
    Remove {
        instance: String,
        file: String,
    },
}

#[derive(Subcommand)]
enum ServerCmd {
    /// Create (and download) a server: vanilla, fabric, forge, neoforge or paper
    Create {
        name: String,
        /// Minecraft version (ignored with --from-instance)
        #[arg(default_value = "latest")]
        minecraft: String,
        #[arg(long, default_value = "vanilla")]
        kind: String,
        /// Same version, loader and mods as this instance (client-only mods are left out)
        #[arg(long)]
        from_instance: Option<String>,
        /// You accept the Minecraft EULA (https://aka.ms/MinecraftEULA); required to start it
        #[arg(long)]
        accept_eula: bool,
    },
    List,
    /// Run a server in this terminal (type commands like `op Steve`; Ctrl+C saves and stops)
    Start {
        name: String,
        /// Also share it with friends over the internet
        #[arg(long)]
        share: bool,
    },
    Delete {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ShareCmd {
    /// Share a world you opened to LAN (the port Minecraft printed, e.g. "Local game hosted on port 51234")
    Host {
        port: u16,
        /// Let everyone with the code in without asking
        #[arg(long)]
        yes: bool,
        /// Also forward this UDP port (e.g. a voice chat mod's); repeatable
        #[arg(long)]
        udp: Vec<u16>,
        /// Also forward this TCP port; repeatable
        #[arg(long)]
        tcp: Vec<u16>,
        /// Offer friends this instance's mods
        #[arg(long)]
        mods_from: Option<String>,
    },
    /// Join a friend's world with their code
    Join {
        /// The four words (or full code) from the host
        #[arg(required = true, num_args = 1..)]
        code: Vec<String>,
        /// Name shown in Minecraft's LAN list
        #[arg(long, default_value = "Friend's world (netherNode)")]
        name: String,
        /// If the host plays with mods, make an instance with the same mods under this name
        #[arg(long)]
        get_mods: Option<String>,
    },
}

#[derive(Subcommand)]
enum FriendsCmd {
    /// Your friends and requests (as last saved)
    List,
    /// Stay online for friends until Ctrl+C, printing who's online and what they share
    Run {
        /// Print a four-word friend code others can add you with
        #[arg(long)]
        code: bool,
        /// Accept every friend request (for testing)
        #[arg(long)]
        accept_all: bool,
    },
    /// Add a friend by their four-word friend code
    Add {
        #[arg(required = true, num_args = 1..)]
        code: Vec<String>,
    },
    /// Accept a friend request
    Accept { id: String },
    /// Remove a friend
    Remove { id: String },
}

#[derive(Subcommand)]
enum ModpackCmd {
    /// Create an instance from a .mrpack file
    Import {
        file: PathBuf,
        /// Instance name (default: the pack's name)
        #[arg(long)]
        name: Option<String>,
    },
    /// Search Modrinth modpacks
    Search {
        #[arg(default_value = "")]
        query: String,
    },
    /// Install a Modrinth modpack (slug or id) as a new instance
    Install { project: String },
}

fn parse_resolution(s: &str) -> std::result::Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or("use WIDTHxHEIGHT, e.g. 1920x1080")?;
    let parse = |v: &str| {
        v.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (320..=16384).contains(n))
    };
    match (parse(w), parse(h)) {
        (Some(w), Some(h)) => Ok((w, h)),
        _ => Err("width and height must be numbers between 320 and 16384".into()),
    }
}

fn parse_loader(name: &str) -> Result<LoaderKind> {
    LoaderKind::parse(name).ok_or_else(|| {
        anyhow::anyhow!("unknown loader '{name}' (fabric, quilt, forge, neoforge, optifine)")
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // --data-dir wins; otherwise the folders chosen in the app's Settings.
    let paths = match cli.data_dir {
        Some(dir) => Paths::new(dir),
        None => nethernode_core::settings::Settings::load().paths(),
    };

    match cli.command {
        Command::Versions {
            snapshots,
            old,
            limit,
        } => {
            let manifest = version::manifest(&download::client(), &paths).await?;
            println!(
                "latest release: {}   latest snapshot: {}\n",
                manifest.latest.release, manifest.latest.snapshot
            );
            let shown = manifest.versions.iter().filter(|v| match v.kind.as_str() {
                "release" => true,
                "snapshot" => snapshots,
                _ => old,
            });
            for v in shown.take(limit) {
                println!("{:<24} {:<10} {}", v.id, v.kind, &v.release_time[..10]);
            }
        }

        Command::Account(cmd) => {
            let mut accounts = Accounts::load(&paths)?;
            match cmd {
                AccountCmd::Add { name } => {
                    let a = accounts.add(&name)?;
                    accounts.save(&paths)?;
                    println!("added {} ({})", a.name, a.uuid);
                }
                AccountCmd::List => {
                    if accounts.accounts.is_empty() {
                        println!("no accounts; add one with `nethernode account add <name>`");
                    }
                    for a in &accounts.accounts {
                        let mark = if accounts.default.as_deref() == Some(&a.name) {
                            "*"
                        } else {
                            " "
                        };
                        println!("{mark} {:<16} {}", a.name, a.uuid);
                    }
                }
                AccountCmd::Remove { name } => {
                    accounts.remove(&name)?;
                    accounts.save(&paths)?;
                    println!("removed {name}");
                }
                AccountCmd::Default { name } => {
                    accounts.set_default(&name)?;
                    accounts.save(&paths)?;
                    println!("default account: {name}");
                }
                AccountCmd::Skin {
                    name,
                    png,
                    slim,
                    cape,
                } => {
                    let account = accounts.pick(Some(&name))?;
                    let store = nethernode_core::skins::SkinStore::new(&paths);
                    let model = if slim {
                        nethernode_core::skins::Model::Slim
                    } else {
                        nethernode_core::skins::Model::Classic
                    };
                    store.set_own_skin(&account.name, &std::fs::read(&png)?, model)?;
                    if let Some(cape) = cape {
                        store.set_own_cape(&account.name, Some(&std::fs::read(&cape)?))?;
                    }
                    println!("{} now wears {}", account.name, png.display());
                }
                AccountCmd::RemoveSkin { name } => {
                    nethernode_core::skins::SkinStore::new(&paths).remove_own(&name)?;
                    println!("{name} is back to the default skin");
                }
            }
        }

        Command::Instance(cmd) => match cmd {
            InstanceCmd::Create {
                name,
                version,
                max_memory,
                min_memory,
                java,
                low_end,
                no_low_end,
                loader,
                resolution,
                fullscreen,
            } => {
                let loader = loader.as_deref().map(parse_loader).transpose()?;
                let id = resolve_version_alias(&paths, &version).await?;
                let low_end = low_end || (!no_low_end && SystemMemory::current().is_low());
                let mut instance = Instance::create(&paths, &name, &id, low_end)?;
                instance.max_memory_mb = max_memory;
                instance.min_memory_mb = min_memory;
                instance.java_path = java;
                instance.resolution = resolution;
                instance.fullscreen = fullscreen;
                instance.save(&paths)?;
                if let Some(kind) = loader {
                    loader::apply(
                        &download::client(),
                        &paths,
                        &mut instance,
                        kind,
                        None,
                        &progress_bars(),
                    )
                    .await?;
                }
                println!(
                    "created '{}' ({}) at {}",
                    instance.name,
                    instance.version,
                    instance.dir(&paths).display()
                );
                if low_end {
                    println!("applied low-end defaults (render distance 8, 60 FPS cap)");
                }
            }
            InstanceCmd::List => {
                let instances = Instance::list(&paths)?;
                if instances.is_empty() {
                    println!(
                        "no instances; create one with `nethernode instance create <name> <version>`"
                    );
                }
                for i in instances {
                    let memory = i
                        .max_memory_mb
                        .map_or("auto".to_string(), |m| format!("{m} MB"));
                    let loader = i.loader.as_ref().map_or("vanilla".to_string(), |l| {
                        format!("{} {}", l.kind, l.version)
                    });
                    println!(
                        "{:<24} {:<10} {:<24} {memory}",
                        i.name,
                        i.minecraft_version(),
                        loader
                    );
                }
            }
            InstanceCmd::Delete { name, yes } => {
                if !yes {
                    bail!("this deletes '{name}' including its worlds; re-run with --yes");
                }
                Instance::delete(&paths, &name)?;
                println!("deleted {name}");
            }
        },

        Command::Loader(cmd) => {
            let http = download::client();
            match cmd {
                LoaderCmd::Versions {
                    loader: kind,
                    minecraft,
                } => {
                    let kind = parse_loader(&kind)?;
                    let versions = loader::versions(&http, kind, &minecraft).await?;
                    if versions.is_empty() {
                        println!("{kind} has no release for Minecraft {minecraft}");
                    }
                    for v in versions.iter().take(20) {
                        println!("{}{}", v.version, if v.stable { "  (stable)" } else { "" });
                    }
                }
                LoaderCmd::Install {
                    instance,
                    loader: kind,
                    version,
                    jar,
                } => {
                    let kind = parse_loader(&kind)?;
                    let mut instance = Instance::load(&paths, &instance)?;
                    if kind == LoaderKind::OptiFine {
                        let jar = jar.ok_or_else(|| anyhow::anyhow!(
                            "download OptiFine from https://optifine.net/downloads and pass it with --jar"
                        ))?;
                        loader::apply_optifine(
                            &http,
                            &paths,
                            &mut instance,
                            &jar,
                            &progress_bars(),
                        )
                        .await?;
                    } else {
                        loader::apply(
                            &http,
                            &paths,
                            &mut instance,
                            kind,
                            version.as_deref(),
                            &progress_bars(),
                        )
                        .await?;
                    }
                    println!("'{}' now runs {}", instance.name, instance.version);
                }
                LoaderCmd::Remove { instance } => {
                    let mut instance = Instance::load(&paths, &instance)?;
                    loader::remove(&paths, &mut instance)?;
                    println!("'{}' is vanilla {} again", instance.name, instance.version);
                }
            }
        }

        Command::Mods(cmd) => {
            let http = download::client();
            match cmd {
                ModsCmd::Check { instance, fix } => {
                    let instance = Instance::load(&paths, &instance)?;
                    if fix {
                        let (installed, still) = nethernode_core::moddeps::install_missing(
                            &http,
                            &paths,
                            &instance,
                            &progress_bars(),
                        )
                        .await?;
                        for name in installed {
                            println!("installed {name}");
                        }
                        for m in still {
                            println!(
                                "still missing: {} (needed by {})",
                                m.id,
                                m.needed_by.join(", ")
                            );
                        }
                    } else {
                        let missing = nethernode_core::moddeps::missing(&instance.mods_dir(&paths));
                        if missing.is_empty() {
                            println!("no missing dependencies");
                        }
                        for m in missing {
                            println!("missing: {} (needed by {})", m.id, m.needed_by.join(", "));
                        }
                    }
                }
                ModsCmd::List { instance } => {
                    let instance = Instance::load(&paths, &instance)?;
                    for m in mods::list(&paths, &instance)? {
                        let version = m.info.as_ref().map_or("", |i| i.version_number.as_str());
                        let state = if m.enabled { " " } else { "x" };
                        println!(
                            "[{state}] {:<32} {:<20} {}",
                            m.display_name(),
                            version,
                            m.file_name
                        );
                    }
                }
                ModsCmd::Search { instance, query } => {
                    let instance = Instance::load(&paths, &instance)?;
                    let results = mods::search(&http, &instance, &query, 0).await?;
                    for hit in results.hits {
                        println!(
                            "{:<28} {:>11} downloads  {}",
                            hit.slug, hit.downloads, hit.description
                        );
                    }
                }
                ModsCmd::Add { instance, project } => {
                    let instance = Instance::load(&paths, &instance)?;
                    let added =
                        mods::install(&http, &paths, &instance, &project, &progress_bars()).await?;
                    if added.is_empty() {
                        println!("already installed");
                    }
                    for title in added {
                        println!("installed {title}");
                    }
                }
                ModsCmd::OptifineAlternative { instance } => {
                    let instance = Instance::load(&paths, &instance)?;
                    let slugs: Vec<&str> =
                        mods::OPTIFINE_ALTERNATIVE.iter().map(|(s, _)| *s).collect();
                    let (installed, skipped) =
                        mods::install_preset(&http, &paths, &instance, &slugs, &progress_bars())
                            .await?;
                    println!("installed {} mods", installed.len());
                    if !skipped.is_empty() {
                        println!("not available for this version: {}", skipped.join(", "));
                    }
                }
                ModsCmd::AddFile { instance, file } => {
                    let instance = Instance::load(&paths, &instance)?;
                    println!("added {}", mods::add_file(&paths, &instance, &file)?);
                }
                ModsCmd::Enable { instance, file } => {
                    mods::set_enabled(&paths, &Instance::load(&paths, &instance)?, &file, true)?;
                }
                ModsCmd::Disable { instance, file } => {
                    mods::set_enabled(&paths, &Instance::load(&paths, &instance)?, &file, false)?;
                }
                ModsCmd::Remove { instance, file } => {
                    mods::remove(&paths, &Instance::load(&paths, &instance)?, &file)?;
                }
            }
        }

        Command::Modpack(cmd) => {
            let http = download::client();
            let report = |i: &Instance| {
                let loader = i.loader.as_ref().map_or("vanilla".to_string(), |l| {
                    format!("{} {}", l.kind, l.version)
                });
                println!(
                    "created '{}': Minecraft {}, {loader}",
                    i.name,
                    i.minecraft_version()
                );
            };
            match cmd {
                ModpackCmd::Import { file, name } => {
                    let i = modpack::import(
                        &http,
                        &paths,
                        &file,
                        name.as_deref(),
                        None,
                        &progress_bars(),
                    )
                    .await?;
                    report(&i);
                }
                ModpackCmd::Search { query } => {
                    for hit in modpack::search(&http, &query, 0).await?.hits {
                        println!(
                            "{:<32} {:>11} downloads  {}",
                            hit.slug, hit.downloads, hit.description
                        );
                    }
                }
                ModpackCmd::Install { project } => {
                    let i = modpack::install(&http, &paths, &project, &progress_bars()).await?;
                    report(&i);
                }
            }
        }

        Command::Server(cmd) => {
            use nethernode_core::server::{self, Server, ServerKind};
            let http = download::client();
            match cmd {
                ServerCmd::Create {
                    name,
                    minecraft,
                    kind,
                    from_instance,
                    accept_eula,
                } => {
                    let source = from_instance
                        .as_deref()
                        .map(|i| Instance::load(&paths, i))
                        .transpose()?;
                    let mut created = match &source {
                        Some(instance) => Server::create_from_instance(&paths, &name, instance)?,
                        None => {
                            let kind = ServerKind::ALL
                                .into_iter()
                                .find(|k| k.name().eq_ignore_ascii_case(&kind))
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "unknown kind '{kind}' (vanilla, fabric, forge, neoforge, paper)"
                                    )
                                })?;
                            let minecraft = resolve_version_alias(&paths, &minecraft).await?;
                            Server::create(&paths, &name, &minecraft, kind)?
                        }
                    };
                    let kind = created.kind;
                    let minecraft = created.minecraft.clone();
                    if let Err(e) =
                        server::install(&http, &paths, &mut created, &progress_bars()).await
                    {
                        let _ = Server::delete(&paths, &name);
                        return Err(e.into());
                    }
                    if let Some(instance) = &source {
                        let skipped =
                            server::copy_instance_mods(&http, &paths, &created, instance).await?;
                        if !skipped.is_empty() {
                            println!("left out client-only mods: {}", skipped.join(", "));
                        }
                    }
                    created.add_to_multiplayer_lists(&paths);
                    if accept_eula {
                        server::accept_eula(&paths, &created)?;
                    }
                    println!(
                        "created {} server '{}' for Minecraft {minecraft} on port {}",
                        kind.name(),
                        created.name,
                        created.port
                    );
                    if !accept_eula {
                        println!(
                            "before starting it, read {} and re-create with --accept-eula, or accept it in the app",
                            server::EULA_URL
                        );
                    }
                }
                ServerCmd::List => {
                    for s in Server::list(&paths) {
                        println!(
                            "{:<24} {:<8} {:<10} port {}",
                            s.name,
                            s.kind.name(),
                            s.minecraft,
                            s.port
                        );
                    }
                }
                ServerCmd::Delete { name, yes } => {
                    if !yes {
                        bail!("this deletes '{name}' including its world; re-run with --yes");
                    }
                    Server::delete(&paths, &name)?;
                    println!("deleted {name}");
                }
                ServerCmd::Start { name, share } => {
                    let srv = Server::load(&paths, &name)?;
                    let cmd = server::command(&http, &paths, &srv, &progress_bars()).await?;
                    println!(
                        "starting {} on port {} with up to {} MB (Ctrl+C saves and stops)",
                        srv.name, srv.port, cmd.heap_mb
                    );
                    let (tx, rx) = futures::channel::mpsc::unbounded();
                    // Lines typed here go to the server console.
                    std::thread::spawn(move || {
                        for line in std::io::stdin().lines().map_while(|l| l.ok()) {
                            if tx.unbounded_send(line).is_err() {
                                break;
                            }
                        }
                    });
                    srv.add_to_multiplayer_lists(&paths);
                    let options = share::HostOptions {
                        lan_port: srv.port,
                        title: srv.name.clone(),
                        extra_ports: srv.ports_for_friends(&paths),
                        manifest: server::friend_manifest(&http, &paths, &srv).await,
                        skins: nethernode_core::skins::for_sharing(&paths),
                    };
                    let sharing = async move {
                        if !share {
                            return std::future::pending::<()>().await;
                        }
                        let result = share::host(
                            options,
                            |event| match event {
                                share::HostEvent::Ready { code, short } => match short {
                                    Some(short) => println!("\nfriends can join with this code:\n  {short}\n(or the full code: {code})\n"),
                                    None => println!("\nfriends can join with this code:\n  {code}\n"),
                                },
                                share::HostEvent::Request { friend, reply, .. } => {
                                    println!("{friend} wants to join; letting them in (CLI)");
                                    let _ = reply.send(true);
                                }
                                share::HostEvent::Joined { friend } => {
                                    println!("{friend} connected")
                                }
                                share::HostEvent::Left { friend } => {
                                    println!("{friend} disconnected")
                                }
                            },
                            std::future::pending::<()>(),
                        )
                        .await;
                        if let Err(e) = result {
                            eprintln!("sharing failed: {e}");
                        }
                        std::future::pending::<()>().await
                    };
                    let output = Arc::new(Mutex::new(Vec::new()));
                    let collected = output.clone();
                    let run = server::run(
                        &cmd,
                        move |line| {
                            println!("{line}");
                            let mut lines = collected.lock().unwrap();
                            if lines.len() > 400 {
                                lines.remove(0);
                            }
                            lines.push(line);
                        },
                        rx,
                        async {
                            let _ = shutdown_signal().await;
                        },
                    );
                    let status = tokio::select! {
                        status = run => status?,
                        () = sharing => unreachable!(),
                    };
                    println!("server stopped ({status})");
                    let lines = output.lock().unwrap().clone();
                    let never_ready = !lines.iter().any(|l| l.contains("Done ("));
                    if (never_ready || !status.success())
                        && let Some(why) = server::diagnose(&srv.dir(&paths), &lines)
                    {
                        println!("{why}");
                    }
                }
            }
        }

        Command::SkinService => {
            let (port, run) = nethernode_core::skins::serve(&paths).await?;
            println!("skin service on http://127.0.0.1:{port}/");
            tokio::select! {
                () = run => {}
                _ = shutdown_signal() => {}
            }
        }
        Command::Share(cmd) => match cmd {
            ShareCmd::Host {
                port,
                yes,
                udp,
                tcp,
                mods_from,
            } => {
                let http = download::client();
                let stop = async {
                    let _ = shutdown_signal().await;
                };
                let mut extra_ports: Vec<share::ExtraPort> = udp
                    .iter()
                    .map(|&port| share::ExtraPort { udp: true, port })
                    .chain(
                        tcp.iter()
                            .map(|&port| share::ExtraPort { udp: false, port }),
                    )
                    .collect();
                let mut manifest = None;
                if let Some(name) = &mods_from {
                    let instance = Instance::load(&paths, name)?;
                    for p in share::detect_mod_ports(&instance.game_dir(&paths), port) {
                        if !extra_ports.contains(&p) {
                            extra_ports.push(p);
                        }
                    }
                    let built = nethernode_core::modsync::build(
                        &http,
                        instance.minecraft_version(),
                        instance.loader.as_ref(),
                        &instance.mods_dir(&paths),
                    )
                    .await?;
                    manifest = Some((built, instance.mods_dir(&paths)));
                }
                let options = share::HostOptions {
                    lan_port: port,
                    title: mods_from.clone().unwrap_or_else(|| "Shared world".into()),
                    extra_ports,
                    manifest,
                    skins: nethernode_core::skins::for_sharing(&paths),
                };
                share::host(
                    options,
                    move |event| match event {
                        share::HostEvent::Ready { code, short } => match short {
                            Some(short) => println!("Sharing localhost:{port}. Give your friend this code:\n\n  {short}\n\n(or the full code: {code})"),
                            None => println!("Sharing localhost:{port}. Give your friend this code:\n\n  {code}\n"),
                        },
                        share::HostEvent::Request { friend, reply, .. } => {
                            let allow = yes || {
                                println!("{friend} wants to join. Allow? [y/N]");
                                let mut line = String::new();
                                std::io::stdin().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("y")
                            };
                            let _ = reply.send(allow);
                        }
                        share::HostEvent::Joined { friend } => println!("{friend} joined"),
                        share::HostEvent::Left { friend } => println!("{friend} left"),
                    },
                    stop,
                )
                .await?;
            }
            ShareCmd::Join {
                code,
                name,
                get_mods,
            } => {
                let code = nethernode_core::invite::resolve(&code.join(" ")).await?;
                let stop = async {
                    let _ = shutdown_signal().await;
                };
                let paths2 = paths.clone();
                share::join(
                    &code,
                    &name,
                    nethernode_core::skins::for_sharing(&paths),
                    None,
                    move |event| match event {
                        share::JoinEvent::Connecting => println!("connecting…"),
                        share::JoinEvent::WaitingForApproval => println!("waiting for your friend to allow you in…"),
                        share::JoinEvent::Ready { port, title, blocked } => {
                            println!(
                                "connected to {title}! In Minecraft: Multiplayer, the world shows under LAN, or Direct Connect to localhost:{port}"
                            );
                            for p in blocked {
                                println!("couldn't open {p} here (another program uses it)");
                            }
                        }
                        share::JoinEvent::Mods(manifest, source) => {
                            println!("your friend plays with mods: {}", manifest.summary());
                            if let Some(found) = nethernode_core::modsync::find_matching(&paths2, &manifest) {
                                println!("instance '{}' already has them", found.name);
                                return;
                            }
                            let Some(target) = get_mods.clone() else {
                                println!("re-run with --get-mods <new instance name> to get the same mods");
                                return;
                            };
                            let paths = paths2.clone();
                            tokio::spawn(async move {
                                let http = download::client();
                                let fetch = |sha1: String| {
                                    let source = source.clone();
                                    async move { source.fetch(sha1).await }
                                };
                                match nethernode_core::modsync::create_instance(
                                    &http, &paths, &manifest, &target, fetch, &progress_bars(),
                                )
                                .await
                                {
                                    Ok(i) => println!("instance '{}' is ready with the same mods", i.name),
                                    Err(e) => eprintln!("couldn't get the mods: {e}"),
                                }
                            });
                        }
                    },
                    stop,
                )
                .await?;
            }
        },

        Command::Friends(cmd) => {
            use nethernode_core::friends::Friends;
            let me = Accounts::load(&paths)?
                .default
                .unwrap_or_else(|| "Player".into());
            match cmd {
                FriendsCmd::List => {
                    let friends = Friends::start(&paths, &me, || {}).await?;
                    println!("your friend id: {}", friends.id());
                    for (f, _) in friends.list() {
                        let state = if f.confirmed {
                            ""
                        } else {
                            " (waiting for them to accept)"
                        };
                        println!("{:<20} {}{state}", f.name, f.id);
                    }
                    for r in friends.requests() {
                        println!(
                            "request from {} ({}): accept with `nethernode friends accept {}`",
                            r.name, r.id, r.id
                        );
                    }
                    friends.shutdown().await;
                }
                FriendsCmd::Add { code } => {
                    let friends = Friends::start(&paths, &me, || {}).await?;
                    let added = friends.add(&code.join(" ")).await?;
                    println!("sent a friend request to {} ({})", added.name, added.id);
                    friends.shutdown().await;
                }
                FriendsCmd::Accept { id } => {
                    let friends = Friends::start(&paths, &me, || {}).await?;
                    friends.accept(&id).await?;
                    println!("accepted");
                    friends.shutdown().await;
                }
                FriendsCmd::Remove { id } => {
                    let friends = Friends::start(&paths, &me, || {}).await?;
                    friends.remove(&id).await;
                    println!("removed");
                    friends.shutdown().await;
                }
                FriendsCmd::Run { code, accept_all } => {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    let friends = Friends::start(&paths, &me, move || {
                        let _ = tx.send(());
                    })
                    .await?;
                    println!("online as {me} ({})", friends.id());
                    if code {
                        println!("friend code: {}", friends.friend_code().await?);
                    }
                    let mut last = String::new();
                    loop {
                        tokio::select! {
                            _ = rx.recv() => {}
                            _ = shutdown_signal() => break,
                        }
                        if accept_all {
                            for r in friends.requests() {
                                let _ = friends.accept(&r.id).await;
                            }
                        }
                        let now = friends
                            .list()
                            .iter()
                            .map(|(f, p)| {
                                let sharing: Vec<String> = p
                                    .sharing
                                    .iter()
                                    .map(|s| format!("{} [{}]", s.title, s.code))
                                    .collect();
                                format!(
                                    "{} {}{}{}",
                                    f.name,
                                    if p.online { "online" } else { "offline" },
                                    if f.confirmed { "" } else { " (waiting)" },
                                    if sharing.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" sharing {}", sharing.join(", "))
                                    }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if now != last {
                            println!("--\n{now}");
                            last = now;
                        }
                    }
                    friends.shutdown().await;
                }
            }
        }

        Command::Launch {
            instance,
            account,
            world,
            server,
            dry_run,
        } => {
            let instance = Instance::load(&paths, &instance)?;
            let _lock = instance.lock(&paths)?;
            let account = Accounts::load(&paths)?.pick(account.as_deref())?;
            let quick_play = world
                .map(QuickPlay::Singleplayer)
                .or(server.map(QuickPlay::Multiplayer));

            let cmd =
                launch::prepare(&paths, &instance, &account, quick_play, &progress_bars()).await?;
            if dry_run {
                println!("cd {}", cmd.cwd.display());
                println!("{} {}", cmd.java.display(), cmd.args.join(" "));
                return Ok(());
            }
            if let Some(warning) = &cmd.heap.warning {
                eprintln!("warning: {warning}");
            }
            println!(
                "launching {} as {} (heap {}-{} MB{})",
                instance.version,
                account.name,
                cmd.heap.min_mb,
                cmd.heap.max_mb,
                if cmd.heap.low_memory {
                    ", low-memory mode"
                } else {
                    ""
                }
            );
            let signal = std::sync::OnceLock::new();
            let outcome = launch::run(&cmd, |line| println!("{line}"), async {
                let _ = signal.set(shutdown_signal().await);
            })
            .await?;
            match outcome {
                launch::RunOutcome::Exited { crash: None, .. } => {}
                launch::RunOutcome::Exited {
                    crash: Some(crash), ..
                } => {
                    eprintln!("\n{}", crash.title);
                    if let Some(cause) = &crash.cause {
                        eprintln!("  cause:  {cause}");
                    }
                    if let Some(report) = &crash.report {
                        eprintln!("  report: {}", report.display());
                    }
                    if let Some(hint) = &crash.hint {
                        eprintln!("\n{hint}");
                    }
                    std::process::exit(crash.exit_code.filter(|c| *c != 0).unwrap_or(1));
                }
                launch::RunOutcome::Stopped => {
                    let signal = signal.get().copied().unwrap_or("signal");
                    bail!("received {signal}, game stopped");
                }
            }
        }
    }
    Ok(())
}

/// Ctrl+C, terminal closed or `kill`: the game must not outlive the launcher.
async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        let mut hup = signal(SignalKind::hangup()).expect("SIGHUP handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "Ctrl+C",
            _ = term.recv() => "SIGTERM",
            _ = hup.recv() => "SIGHUP",
        }
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows;
        let mut close = windows::ctrl_close().expect("console close handler");
        let mut brk = windows::ctrl_break().expect("Ctrl+Break handler");
        let mut logoff = windows::ctrl_logoff().expect("logoff handler");
        let mut shutdown = windows::ctrl_shutdown().expect("shutdown handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "Ctrl+C",
            _ = close.recv() => "console closed",
            _ = brk.recv() => "Ctrl+Break",
            _ = logoff.recv() => "logoff",
            _ = shutdown.recv() => "shutdown",
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "Ctrl+C"
    }
}

async fn resolve_version_alias(paths: &Paths, version: &str) -> Result<String> {
    if version != "latest" && version != "snapshot" {
        return Ok(version.to_string());
    }
    let manifest = version::manifest(&download::client(), paths).await?;
    Ok(if version == "latest" {
        manifest.latest.release
    } else {
        manifest.latest.snapshot
    })
}

/// One progress bar per download stage.
fn progress_bars() -> ProgressFn {
    let current: Mutex<Option<ProgressBar>> = Mutex::new(None);
    let style = ProgressStyle::with_template("{msg:>24} [{bar:40}] {pos}/{len}")
        .expect("valid template")
        .progress_chars("=> ");
    Arc::new(move |event| {
        let mut current = current.lock().unwrap();
        match event {
            Progress::Stage { name, total } => {
                if let Some(bar) = current.take() {
                    bar.finish();
                }
                let bar = ProgressBar::new(total)
                    .with_style(style.clone())
                    .with_message(name);
                *current = Some(bar);
            }
            Progress::Advance => {
                if let Some(bar) = current.as_ref() {
                    bar.inc(1);
                }
            }
        }
    })
}
