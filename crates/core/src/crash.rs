//! Working out why the game stopped: Minecraft crash reports, JVM fatal-error logs and the
//! last lines of output, plus a plain-language hint for the causes we recognise.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What went wrong, for showing to the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashInfo {
    /// One line, e.g. "Minecraft crashed while Initializing game".
    pub title: String,
    /// The underlying error, e.g. "java.lang.OutOfMemoryError: Java heap space".
    pub cause: Option<String>,
    /// What the player can do about it, when we recognise the problem.
    pub hint: Option<String>,
    /// The crash report or JVM error log, if one was written.
    pub report: Option<PathBuf>,
    pub exit_code: Option<i32>,
}

/// Known problems: (text to look for, advice). Checked in order; the first match wins, so
/// specific patterns go before general ones.
const HINTS: &[(&str, &str)] = &[
    (
        "getAvailableDisplayModes",
        "Minecraft 1.12.2 and older need the `xrandr` program on Linux. Install it (Fedora: \
         sudo dnf install xrandr, Debian/Ubuntu: sudo apt install x11-xserver-utils, \
         Arch: sudo pacman -S xorg-xrandr) and try again.",
    ),
    (
        "Could not reserve enough space for",
        "Java couldn't get the memory it was asked for. Lower the maximum memory in the \
         instance settings. If you chose a custom Java, make sure it is a 64-bit Java.",
    ),
    (
        "Invalid maximum heap size",
        "The memory setting is invalid. Set the instance's memory back to automatic or to a \
         plain number of MB.",
    ),
    (
        "Invalid initial heap size",
        "The memory setting is invalid. Set the instance's memory back to automatic or to a \
         plain number of MB.",
    ),
    (
        "Initial heap size set to a larger value than the maximum",
        "The minimum memory is set higher than the maximum. Fix the -Xms/-Xmx values in the \
         instance's extra JVM arguments, or remove them to use automatic memory.",
    ),
    (
        "OutOfMemoryError",
        "The game ran out of memory. Give it more in the instance settings, close other \
         programs, or lower the render distance.",
    ),
    (
        "UnsupportedClassVersionError",
        "This Java is too old for this Minecraft version. Clear the custom Java path in the \
         instance settings so netherNode picks the right one automatically.",
    ),
    (
        "Unrecognized VM option",
        "One of the extra JVM arguments isn't supported by this Java. Remove it in the \
         instance settings.",
    ),
    (
        "Error: Could not find or load main class",
        "Game files are missing or damaged. Launch again to re-download them.",
    ),
    (
        "Pixel format not accelerated",
        "Your graphics driver doesn't provide OpenGL acceleration. Install or update the GPU \
         driver from your graphics card maker (on Linux, install Mesa's OpenGL drivers).",
    ),
    (
        "GLFW error 65542",
        "Your graphics driver doesn't support OpenGL. Install or update the GPU driver \
         (on Linux, install Mesa's OpenGL drivers).",
    ),
    (
        "GLFW error 65543",
        "Your graphics card or driver doesn't support the OpenGL version this Minecraft needs. \
         Update the GPU driver, or play an older Minecraft version.",
    ),
    (
        "does not appear to support OpenGL",
        "Your graphics driver doesn't support OpenGL. Install or update the GPU driver.",
    ),
    (
        "UnsatisfiedLinkError",
        "A native library couldn't be loaded. This usually means this Minecraft version has no \
         build for your CPU or OS. Try launching again, or pick a newer version.",
    ),
    (
        "Missing or unsupported mandatory dependencies",
        "A mod needs another mod that isn't installed, and netherNode couldn't find it on \
         Modrinth for this version. The report names it (\"Mod ID: …\"); get it from the mod's \
         own page and drop the jar on the Mods screen.",
    ),
    (
        "which is missing!",
        "A mod needs another mod that isn't installed, and netherNode couldn't find it on \
         Modrinth for this version. The report names it; get it from the mod's own page and \
         drop the jar on the Mods screen.",
    ),
    (
        "Incompatible mods found",
        "Some mods are incompatible with each other or with this Minecraft version. The report \
         lists which ones.",
    ),
];

/// JVM crashes inside these libraries are graphics-driver crashes.
const DRIVER_LIBRARIES: &[&str] = &[
    "atio6axx",
    "atioglxx",
    "ig9icd",
    "ig7icd",
    "ig75icd",
    "igxelpicd",
    "nvoglv",
    "libnvidia",
    "radeonsi",
    "iris_dri",
    "libGLX",
    "libGL.so",
    "amdgpu",
    "OpenGL.framework",
];

fn hint_for(text: &str) -> Option<String> {
    HINTS
        .iter()
        .find(|(needle, _)| text.contains(needle))
        .map(|(_, hint)| hint.to_string())
}

/// Java's catch-all lines, which never say what actually went wrong.
const GENERIC_ERRORS: &[&str] = &[
    "Error occurred during initialization of VM",
    "Could not create the Java Virtual Machine",
    "A fatal exception has occurred",
];

/// The most informative error line in the output: one we have a hint for, else the last
/// error-looking line that isn't one of Java's generic ones.
fn cause_from_tail(tail: &[String]) -> Option<String> {
    let known = tail
        .iter()
        .rev()
        .find(|l| HINTS.iter().any(|(needle, _)| l.contains(needle)));
    let fallback = || {
        tail.iter().rev().find(|l| {
            (l.contains("Error") || l.contains("Exception") || l.contains("Invalid"))
                && !GENERIC_ERRORS.iter().any(|g| l.contains(g))
        })
    };
    known.or_else(fallback).map(|l| l.trim().to_string())
}

/// The newest file in `dir` whose name passes `matches` and which was written at or after `since`.
fn newest_since(dir: &Path, since: SystemTime, matches: impl Fn(&str) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(&matches))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(modified, _)| *modified >= since)
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// Pull the description and root cause out of a Minecraft crash report.
fn parse_crash_report(text: &str) -> (Option<String>, Option<String>) {
    let description = text
        .lines()
        .find_map(|l| l.strip_prefix("Description: "))
        .map(|d| d.trim().to_string());
    // The first stack trace follows the description; its last "Caused by" is the root cause.
    let trace: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.starts_with("Description: "))
        .skip(1)
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && !l.starts_with("A detailed walkthrough"))
        .collect();
    let cause = trace
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("Caused by: "))
        .or_else(|| trace.first().copied())
        .map(|c| c.trim().to_string());
    (description, cause)
}

/// The "Problematic frame" line of a JVM fatal-error log.
fn parse_jvm_error(text: &str) -> Option<String> {
    let mut lines = text.lines();
    lines.find(|l| l.starts_with("# Problematic frame:"))?;
    lines
        .next()
        .map(|l| l.trim_start_matches('#').trim().to_string())
}

/// Decide whether the run crashed and why. Returns `None` for a normal exit.
///
/// `started` is when the game was launched, so older reports are ignored. `log_tail` is the
/// last lines of output, used when the JVM died before Minecraft could write a report.
pub fn analyze(
    game_dir: &Path,
    started: SystemTime,
    exit_code: Option<i32>,
    log_tail: &[String],
) -> Option<CrashInfo> {
    // Allow for coarse file-time resolution on some filesystems.
    let since = started - std::time::Duration::from_secs(2);
    let report = newest_since(&game_dir.join("crash-reports"), since, |n| {
        n.starts_with("crash-")
    });
    let jvm_log = newest_since(game_dir, since, |n| {
        n.starts_with("hs_err_pid") && n.ends_with(".log")
    });
    let failed = exit_code != Some(0);
    if !failed && report.is_none() && jvm_log.is_none() {
        return None;
    }
    let tail = log_tail.join("\n");

    if let Some(path) = report {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (description, cause) = parse_crash_report(&text);
        return Some(CrashInfo {
            title: match description {
                Some(d) => format!("Minecraft crashed: {d}"),
                None => "Minecraft crashed".into(),
            },
            hint: hint_for(&text).or_else(|| hint_for(&tail)),
            cause,
            report: Some(path),
            exit_code,
        });
    }

    if let Some(path) = jvm_log {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let frame = parse_jvm_error(&text);
        let driver = frame
            .as_deref()
            .is_some_and(|f| DRIVER_LIBRARIES.iter().any(|lib| f.contains(lib)));
        return Some(CrashInfo {
            title: "Java crashed".into(),
            hint: if driver {
                Some("Java crashed inside the graphics driver. Update your GPU driver.".into())
            } else {
                hint_for(&text).or_else(|| hint_for(&tail))
            },
            cause: frame,
            report: Some(path),
            exit_code,
        });
    }

    // No report: the JVM most likely refused to start.
    let cause = cause_from_tail(log_tail);
    Some(CrashInfo {
        title: match exit_code {
            Some(code) => format!("The game stopped unexpectedly (exit code {code})"),
            None => "The game was terminated".into(),
        },
        hint: hint_for(&tail),
        cause,
        report: None,
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "---- Minecraft Crash Report ----
// Who set us up the TNT?

Time: 10/21/26 9:08 PM
Description: Initializing game

java.lang.ExceptionInInitializerError
\tat ave.ar(SourceFile:560)
Caused by: java.lang.ArrayIndexOutOfBoundsException: 0
\tat org.lwjgl.opengl.LinuxDisplay.getAvailableDisplayModes(LinuxDisplay.java:951)
\t... 4 more


A detailed walkthrough of the error, its code path and all known details is as follows:
---------------------------------------------------------------------------------------
\tOpenGL: ~~ERROR~~ RuntimeException: No OpenGL context found in the current thread.
";

    #[test]
    fn parses_crash_report() {
        let (description, cause) = parse_crash_report(REPORT);
        assert_eq!(description.as_deref(), Some("Initializing game"));
        assert_eq!(
            cause.as_deref(),
            Some("java.lang.ArrayIndexOutOfBoundsException: 0")
        );
    }

    #[test]
    fn crash_without_caused_by_uses_first_line() {
        let (_, cause) = parse_crash_report(
            "Description: Ticking\n\njava.lang.NullPointerException: x\n\tat a.b\n",
        );
        assert_eq!(cause.as_deref(), Some("java.lang.NullPointerException: x"));
    }

    #[test]
    fn specific_hint_beats_general_one() {
        // The xrandr crash also mentions OpenGL; the xrandr advice must win.
        assert!(hint_for(REPORT).unwrap().contains("xrandr"));
        assert!(
            hint_for("java.lang.OutOfMemoryError: Java heap space")
                .unwrap()
                .contains("memory")
        );
        assert!(hint_for("all good").is_none());
    }

    #[test]
    fn parses_jvm_error_frame() {
        let log = "#\n# A fatal error has been detected by the Java Runtime Environment:\n#\n\
                   # Problematic frame:\n# C  [atio6axx.dll+0x1234]\n#\n";
        assert_eq!(
            parse_jvm_error(log).as_deref(),
            Some("C  [atio6axx.dll+0x1234]")
        );
    }

    fn temp_game_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nethernode-crash-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("crash-reports")).unwrap();
        dir
    }

    #[test]
    fn finds_new_report_and_ignores_clean_exit() {
        let dir = temp_game_dir("report");
        let started = SystemTime::now();
        assert_eq!(analyze(&dir, started, Some(0), &[]), None);

        std::fs::write(
            dir.join("crash-reports/crash-2026-10-21_21.08.38-client.txt"),
            REPORT,
        )
        .unwrap();
        let info = analyze(&dir, started, Some(255), &[]).unwrap();
        assert_eq!(info.title, "Minecraft crashed: Initializing game");
        assert!(info.report.is_some() && info.hint.unwrap().contains("xrandr"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jvm_refusing_to_start() {
        let dir = temp_game_dir("jvm");
        let tail = [
            "Error occurred during initialization of VM".to_string(),
            "Could not reserve enough space for 99999999KB object heap".to_string(),
        ];
        let info = analyze(&dir, SystemTime::now(), Some(1), &tail).unwrap();
        assert_eq!(info.title, "The game stopped unexpectedly (exit code 1)");
        assert!(info.hint.unwrap().contains("Lower the maximum memory"));
        // The specific line, not Java's generic "Error occurred during initialization of VM".
        assert!(info.cause.unwrap().starts_with("Could not reserve"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
