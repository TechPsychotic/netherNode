use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use nethernode_core::account::Accounts;
use nethernode_core::download::{self, Progress, ProgressFn};
use nethernode_core::instance::Instance;
use nethernode_core::launch::{self, QuickPlay};
use nethernode_core::memory::SystemMemory;
use nethernode_core::paths::Paths;
use nethernode_core::version;

#[derive(Parser)]
#[command(name = "nethernode", version, about = "Offline Minecraft launcher")]
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::new(cli.data_dir.unwrap_or_else(Paths::default_root));

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
            } => {
                let id = resolve_version_alias(&paths, &version).await?;
                let low_end = low_end || (!no_low_end && SystemMemory::current().is_low());
                let mut instance = Instance::create(&paths, &name, &id, low_end)?;
                instance.max_memory_mb = max_memory;
                instance.min_memory_mb = min_memory;
                instance.java_path = java;
                instance.save(&paths)?;
                println!(
                    "created '{}' ({}) at {}",
                    instance.name,
                    instance.version,
                    instance.dir(&paths).display()
                );
                if low_end {
                    println!(
                        "applied low-end graphics defaults (render distance 6, 60 FPS cap, fast graphics)"
                    );
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
                    println!("{:<24} {:<16} {memory}", i.name, i.version);
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
