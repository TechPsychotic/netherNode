//! Turning a resolved + installed version into a `java ...` command line.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::account::Account;
use crate::crash::CrashInfo;
use crate::download::{self, ProgressFn};
use crate::error::Result;
use crate::install::{self, Installed};
use crate::instance::Instance;
use crate::java;
use crate::log::{LogLine, XmlLogParser};
use crate::memory::{self, HeapPlan, SystemMemory};
use crate::paths::Paths;
use crate::rules::{self, Environment};
use crate::version::{self, ArgValue, Argument, JavaVersion, VersionJson};

pub const LAUNCHER_NAME: &str = "netherNode";
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Join a world or server directly after the game starts.
#[derive(Debug, Clone)]
pub enum QuickPlay {
    Singleplayer(String),
    /// `host` or `host:port`.
    Multiplayer(String),
}

#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub heap: HeapPlan,
    /// Where the main class sits in `args`; JVM options go before it.
    pub main_class_at: usize,
    /// authlib-injector's jar and the data folder whose skins to serve, when skins are on.
    pub skins: Option<(PathBuf, Paths)>,
    /// Java's major version, if known (argument files need Java 9+).
    pub java_major: Option<u32>,
}

/// Windows refuses command lines longer than 32,767 characters ("The filename or extension
/// is too long"), which big modpacks' classpaths exceed. Stay well below it.
const MAX_COMMAND_LINE: usize = 24_000;

/// How to pass the arguments when the command line would be too long.
#[derive(Debug, PartialEq, Eq)]
enum LongCommand {
    /// Short enough: pass as is.
    Direct,
    /// Java 9+: put every argument in a file and pass `@file`.
    ArgFile(String),
    /// Java 8: move the classpath to the CLASSPATH variable (it has its own limit).
    ClasspathEnv {
        args: Vec<String>,
        classpath: String,
    },
}

fn fit_command_line(args: &[String], java_major: Option<u32>, limit: usize) -> LongCommand {
    let length: usize = args.iter().map(|a| a.len() + 3).sum();
    if length <= limit {
        return LongCommand::Direct;
    }
    if java_major.is_some_and(|m| m >= 9) {
        // One argument per line, quoted; Java's argument files treat \ as an escape.
        let file = args
            .iter()
            .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join("\n");
        return LongCommand::ArgFile(file);
    }
    match args.iter().position(|a| a == "-cp" || a == "-classpath") {
        Some(i) if i + 1 < args.len() => {
            let mut rest = args.to_vec();
            let classpath = rest.remove(i + 1);
            rest.remove(i);
            LongCommand::ClasspathEnv {
                args: rest,
                classpath,
            }
        }
        _ => LongCommand::Direct,
    }
}

impl LaunchCommand {
    /// Add JVM options (they must come before the main class).
    pub fn add_jvm_args(&mut self, extra: impl IntoIterator<Item = String>) {
        let at = self.main_class_at.min(self.args.len());
        let before = self.args.len();
        self.args.splice(at..at, extra);
        self.main_class_at += self.args.len() - before;
    }

    pub fn to_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.java);
        let limit = if std::env::var_os("NETHERNODE_FORCE_ARGFILE").is_some() {
            0 // diagnostics: exercise the long-command path anywhere
        } else if cfg!(windows) {
            MAX_COMMAND_LINE
        } else {
            1_000_000
        };
        match fit_command_line(&self.args, self.java_major, limit) {
            LongCommand::Direct => {
                cmd.args(&self.args);
            }
            LongCommand::ArgFile(contents) => {
                // Next to the game folder, so it's per instance and never in the way.
                let file = self.cwd.with_file_name("launch-args.txt");
                match std::fs::write(&file, contents) {
                    Ok(()) => {
                        cmd.arg(format!("@{}", file.display()));
                    }
                    Err(_) => {
                        cmd.args(&self.args);
                    }
                }
            }
            LongCommand::ClasspathEnv { args, classpath } => {
                cmd.args(args).env("CLASSPATH", classpath);
            }
        }
        // If the launcher goes away, the game goes with it instead of lingering unseen.
        cmd.current_dir(&self.cwd).kill_on_drop(true);
        // Without this, a GUI launcher (no console of its own) would pop up a console window.
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd
    }
}

/// How a game run ended.
#[derive(Debug)]
pub enum RunOutcome {
    /// The game ended by itself. `crash` is set when it crashed or failed to start.
    Exited {
        status: std::process::ExitStatus,
        crash: Option<CrashInfo>,
    },
    /// `stop` fired and the game was killed.
    Stopped,
}

/// Start the game and feed every log line (stdout and stderr) to `on_line` until it exits.
/// If `stop` completes first, the game is killed. Shared by the CLI and the GUI.
pub async fn run(
    cmd: &LaunchCommand,
    mut on_line: impl FnMut(LogLine),
    stop: impl std::future::Future<Output = ()>,
) -> Result<RunOutcome> {
    use std::collections::VecDeque;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    /// Output lines kept for crash analysis.
    const TAIL: usize = 200;
    let started = std::time::SystemTime::now();
    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL);
    let mut on_line = move |line: LogLine, tail: &mut VecDeque<String>| {
        if tail.len() == TAIL {
            tail.pop_front();
        }
        tail.push_back(line.text.clone());
        on_line(line);
    };

    // Skins: point the game at the local skin service (started on first use).
    let with_skins;
    let cmd = match &cmd.skins {
        Some((injector, paths)) => match crate::skins::ensure_service(paths).await {
            Ok(port) => {
                let mut c = cmd.clone();
                c.add_jvm_args(crate::skins::agent_args(injector, port));
                with_skins = c;
                &with_skins
            }
            Err(e) => {
                let text = format!("skins are off for this run: {e}");
                on_line(
                    LogLine {
                        level: "WARN".into(),
                        thread: "netherNode".into(),
                        text,
                    },
                    &mut tail,
                );
                cmd
            }
        },
        None => cmd,
    };

    let mut child = cmd
        .to_command()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| crate::error::other(format!("failed to start {}: {e}", cmd.java.display())))?;
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut stderr = BufReader::new(child.stderr.take().expect("piped stderr")).lines();
    let mut parser = XmlLogParser::default();
    let (mut out_done, mut err_done) = (false, false);
    tokio::pin!(stop);

    while !(out_done && err_done) {
        tokio::select! {
            line = stdout.next_line(), if !out_done => match line? {
                Some(line) => {
                    if let Some(entry) = parser.feed(&line) {
                        on_line(entry, &mut tail);
                    }
                }
                None => out_done = true,
            },
            line = stderr.next_line(), if !err_done => match line? {
                Some(text) => on_line(LogLine { level: "STDERR".into(), thread: "java".into(), text }, &mut tail),
                None => err_done = true,
            },
            () = &mut stop => {
                child.kill().await?;
                return Ok(RunOutcome::Stopped);
            }
        }
    }
    tokio::select! {
        status = child.wait() => {
            let status = status?;
            let tail: Vec<String> = tail.into();
            let crash = crate::crash::analyze(&cmd.cwd, started, status.code(), &tail);
            Ok(RunOutcome::Exited { status, crash })
        }
        () = &mut stop => {
            child.kill().await?;
            Ok(RunOutcome::Stopped)
        }
    }
}

/// Resolve, download and assemble everything needed to start `instance` as `account`.
pub async fn prepare(
    paths: &Paths,
    instance: &Instance,
    account: &Account,
    quick_play: Option<QuickPlay>,
    progress: &ProgressFn,
) -> Result<LaunchCommand> {
    let http = download::client();
    let version = version::resolve(&http, paths, &instance.version).await?;
    let game_dir = instance.game_dir(paths);
    std::fs::create_dir_all(&game_dir)?;

    let mut env = Environment::current();
    // Always pass a size, so the game doesn't open at its tiny 854x480 default.
    {
        env.features.insert("has_custom_resolution".into());
    }
    match &quick_play {
        Some(QuickPlay::Singleplayer(_)) => {
            env.features.insert("is_quick_play_singleplayer".into())
        }
        Some(QuickPlay::Multiplayer(_)) => env.features.insert("is_quick_play_multiplayer".into()),
        None => false,
    };

    check_lwjgl2_xrandr(&version, &env)?;

    // Java first: an x86-64 runtime emulated on an ARM machine needs x86-64 natives.
    let required = version.java_version.clone().unwrap_or(JavaVersion {
        component: Some("jre-legacy".into()),
        major_version: 8,
    });
    let (java, java_major) = match &instance.java_path {
        Some(path) => {
            // A too-old Java fails with confusing "Unrecognized VM option" errors; say it plainly.
            match java::major_version(path) {
                Some(found) if found < required.major_version => {
                    return Err(crate::error::other(format!(
                        "Minecraft {} needs Java {} or newer, but the Java set for this instance ({}) \
                         is Java {found}. Clear the Java path to let netherNode pick one automatically.",
                        version.id,
                        required.major_version,
                        path.display()
                    )));
                }
                Some(found) => (path.clone(), Some(found)),
                None => {
                    return Err(crate::error::other(format!(
                        "the Java set for this instance ({}) could not be run",
                        path.display()
                    )));
                }
            }
        }
        None => {
            let runtime = java::resolve(&http, paths, &env, &required, progress).await?;
            if let Some(arch) = runtime.arch {
                env.arch = arch;
            }
            // Mojang's runtime for this version is exactly the major version it asks for.
            (runtime.java, Some(required.major_version))
        }
    };

    // Mods that need other mods: fetch what's missing before the game complains.
    if instance
        .loader
        .as_ref()
        .is_some_and(|l| !l.kind.mod_loaders().is_empty())
    {
        progress(download::Progress::Stage {
            name: "Checking mods".into(),
            total: 1,
        });
        match crate::moddeps::install_missing(&http, paths, instance, progress).await {
            Ok((_, still)) => {
                for m in still {
                    eprintln!(
                        "warning: {} needs {}, which isn't on Modrinth for this version",
                        m.needed_by.join(", "),
                        m.id
                    );
                }
            }
            Err(e) => eprintln!("warning: couldn't check mod dependencies: {e}"),
        }
        progress(download::Progress::Advance);
    }

    let natives_dir = instance.dir(paths).join("natives");
    let installed = install::install(
        &http,
        paths,
        &version,
        &env,
        &game_dir,
        &natives_dir,
        progress,
    )
    .await?;

    // Measured after downloads, right before starting, so the free-memory figure is current.
    // -Xmx/-Xms in the instance's own JVM arguments override the memory settings.
    let user_heap = |flag: &str| {
        instance
            .jvm_args
            .iter()
            .rev()
            .find_map(|a| a.strip_prefix(flag).and_then(memory::parse_size_mb))
    };
    let heap = memory::plan_heap(
        SystemMemory::current(),
        user_heap("-Xmx").or(instance.max_memory_mb),
        user_heap("-Xms").or(instance.min_memory_mb),
    );
    let vars = variables(
        paths,
        instance,
        account,
        &version,
        &installed,
        &env,
        quick_play.as_ref(),
    );
    let (args, main_class_at) = build_args(
        instance,
        &version,
        &installed,
        &env,
        &vars,
        quick_play.as_ref(),
        &heap,
    );
    // Skins are a nice-to-have: if authlib-injector can't be fetched, play without them.
    let skins = if crate::settings::Settings::load().skins {
        match crate::skins::ensure_injector(&http, paths).await {
            Ok(jar) => Some((jar, paths.clone())),
            Err(e) => {
                eprintln!("skins are off for this run: {e}");
                None
            }
        }
    } else {
        None
    };
    Ok(LaunchCommand {
        java,
        args,
        cwd: game_dir,
        heap,
        main_class_at,
        skins,
        java_major,
    })
}

/// LWJGL 2 (Minecraft 1.12.2 and older) runs the `xrandr` program on Linux to list display
/// modes and crashes with ArrayIndexOutOfBoundsException when it is missing.
fn check_lwjgl2_xrandr(version: &VersionJson, env: &Environment) -> Result<()> {
    let uses_lwjgl2 = version
        .libraries
        .iter()
        .any(|l| l.name.starts_with("org.lwjgl.lwjgl:lwjgl:"));
    if env.os_name != "linux" || !uses_lwjgl2 {
        return Ok(());
    }
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("xrandr").is_file()));
    if found {
        Ok(())
    } else {
        Err(crate::error::other(format!(
            "Minecraft {} needs the `xrandr` program on Linux, which is not installed.\n\
             Install it with: sudo dnf install xrandr (Fedora), sudo apt install x11-xserver-utils \
             (Debian/Ubuntu) or sudo pacman -S xorg-xrandr (Arch)",
            version.id
        )))
    }
}

fn variables(
    paths: &Paths,
    instance: &Instance,
    account: &Account,
    version: &VersionJson,
    installed: &Installed,
    env: &Environment,
    quick_play: Option<&QuickPlay>,
) -> HashMap<&'static str, String> {
    let path = |p: &PathBuf| p.to_string_lossy().into_owned();
    let separator = if env.os_name == "windows" { ";" } else { ":" };
    let classpath: Vec<String> = installed.classpath.iter().map(path).collect();
    let game_dir = instance.game_dir(paths);
    let (width, height) = instance.window_size();

    let mut vars = HashMap::from([
        ("auth_player_name", account.name.clone()),
        ("auth_uuid", account.uuid.replace('-', "")),
        // Offline: any non-empty token works; the game never checks it for singleplayer/LAN.
        ("auth_access_token", "0".into()),
        ("auth_session", "0".into()),
        ("auth_xuid", "0".into()),
        ("clientid", "0".into()),
        ("user_type", "legacy".into()),
        ("user_properties", "{}".into()),
        ("version_name", version.id.clone()),
        (
            "version_type",
            if version.kind.is_empty() {
                "release".into()
            } else {
                version.kind.clone()
            },
        ),
        ("game_directory", path(&game_dir)),
        ("assets_root", path(&installed.assets_root)),
        ("game_assets", path(&installed.game_assets)),
        ("assets_index_name", installed.assets_index_name.clone()),
        ("natives_directory", path(&installed.natives_dir)),
        ("library_directory", path(&paths.libraries())),
        ("classpath_separator", separator.into()),
        ("classpath", classpath.join(separator)),
        ("launcher_name", LAUNCHER_NAME.into()),
        ("launcher_version", LAUNCHER_VERSION.into()),
        ("resolution_width", width.to_string()),
        ("resolution_height", height.to_string()),
    ]);
    match quick_play {
        Some(QuickPlay::Singleplayer(world)) => vars.insert("quickPlaySingleplayer", world.clone()),
        Some(QuickPlay::Multiplayer(server)) => vars.insert("quickPlayMultiplayer", server.clone()),
        None => None,
    };
    vars
}

/// Replace every `${name}` with its value; unknown placeholders are left as-is.
fn substitute(s: &str, vars: &HashMap<&'static str, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end) => {
                let key = &rest[start + 2..start + end];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + end + 1]),
                }
                rest = &rest[start + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn expand(
    args: &[Argument],
    env: &Environment,
    vars: &HashMap<&'static str, String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Argument::Plain(s) => out.push(substitute(s, vars)),
            Argument::Conditional { rules, value } if rules::allowed(rules, env) => match value {
                ArgValue::One(s) => out.push(substitute(s, vars)),
                ArgValue::Many(v) => out.extend(v.iter().map(|s| substitute(s, vars))),
            },
            Argument::Conditional { .. } => {}
        }
    }
    out
}

fn build_args(
    instance: &Instance,
    version: &VersionJson,
    installed: &Installed,
    env: &Environment,
    vars: &HashMap<&'static str, String>,
    quick_play: Option<&QuickPlay>,
    heap: &HeapPlan,
) -> (Vec<String>, usize) {
    // Heap flags in the instance's own JVM arguments win over ours.
    let user_sets = |flag: &str| instance.jvm_args.iter().any(|a| a.starts_with(flag));
    let mut args = Vec::new();
    if !user_sets("-Xms") {
        args.push(format!("-Xms{}M", heap.min_mb));
    }
    if !user_sets("-Xmx") {
        args.push(format!("-Xmx{}M", heap.max_mb));
    }
    args.extend(memory::gc_flags(heap, &instance.jvm_args));

    let modern = version.arguments.as_ref().filter(|a| !a.jvm.is_empty());
    match modern {
        Some(a) => {
            // Mojang's suggestions assume plenty of RAM (4 GB heap, ZGC, AlwaysPreTouch).
            // Keep the harmless ones and use our own heap and GC settings instead.
            let defaults = expand(&a.default_user_jvm, env, vars);
            args.extend(
                defaults
                    .into_iter()
                    .filter(|s| !memory::is_replaced_flag(s)),
            );
            args.extend(expand(&a.jvm, env, vars));
        }
        None => {
            // Pre-1.13 versions have no JVM arguments in their JSON.
            if env.os_name == "osx" {
                args.push("-XstartOnFirstThread".into());
            }
            for s in [
                "-Djava.library.path=${natives_directory}",
                "-Dminecraft.launcher.brand=${launcher_name}",
                "-Dminecraft.launcher.version=${launcher_version}",
                "-cp",
                "${classpath}",
            ] {
                args.push(substitute(s, vars));
            }
        }
    }
    if let Some(logging) = &installed.logging_arg {
        args.push(logging.clone());
    }
    args.extend(instance.jvm_args.iter().cloned());

    let main_class_at = args.len();
    args.push(version.main_class.clone().unwrap_or_default());

    let game: Vec<String> = match (&version.arguments, &version.minecraft_arguments) {
        (Some(a), _) if !a.game.is_empty() => expand(&a.game, env, vars),
        (_, Some(legacy)) => legacy
            .split_whitespace()
            .map(|s| substitute(s, vars))
            .collect(),
        _ => Vec::new(),
    };
    let supports_quick_play = game.iter().any(|s| s.starts_with("--quickPlay"));
    args.extend(game);

    // Pre-1.13 versions have no resolution arguments in their JSON but accept these.
    if modern.is_none() {
        let (w, h) = instance.window_size();
        args.extend([
            "--width".into(),
            w.to_string(),
            "--height".into(),
            h.to_string(),
        ]);
    }
    if instance.fullscreen {
        args.push("--fullscreen".into());
    }
    // Versions before quick play (pre-1.20) still accept --server/--port.
    if let (Some(QuickPlay::Multiplayer(server)), false) = (quick_play, supports_quick_play) {
        let (host, port) = server.split_once(':').unwrap_or((server, "25565"));
        args.extend(["--server".into(), host.into(), "--port".into(), port.into()]);
    }
    (args, main_class_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_command_lines_use_an_argument_file_or_classpath_variable() {
        let args: Vec<String> = vec![
            "-Xmx2G".into(),
            "-cp".into(),
            "C:\\libs\\a.jar;C:\\libs\\b \"c\".jar".into(),
            "net.minecraft.Main".into(),
        ];
        assert_eq!(
            fit_command_line(&args, Some(21), 10_000),
            LongCommand::Direct
        );
        let LongCommand::ArgFile(file) = fit_command_line(&args, Some(21), 10) else {
            panic!("expected an argument file");
        };
        assert_eq!(
            file.lines().nth(2).unwrap(),
            "\"C:\\\\libs\\\\a.jar;C:\\\\libs\\\\b \\\"c\\\".jar\""
        );
        assert_eq!(
            fit_command_line(&args, Some(8), 10),
            LongCommand::ClasspathEnv {
                args: vec!["-Xmx2G".into(), "net.minecraft.Main".into()],
                classpath: args[2].clone(),
            }
        );
    }

    #[test]
    fn substitutes_placeholders() {
        let vars = HashMap::from([("a", "1".to_string()), ("b", "two".to_string())]);
        assert_eq!(substitute("x${a}y${b}z", &vars), "x1ytwoz");
        assert_eq!(substitute("${missing}", &vars), "${missing}");
        assert_eq!(substitute("-Dp=${a}/java", &vars), "-Dp=1/java");
        assert_eq!(substitute("broken ${a", &vars), "broken ${a");
    }
}
