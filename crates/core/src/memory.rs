//! Heap sizing and JVM flags that keep the game (and the desktop) alive on low-memory machines.

use sysinfo::System;

/// Memory the JVM uses outside the heap: metaspace, code cache, LWJGL/OpenGL buffers, threads.
const NON_HEAP_MB: u64 = 768;
const MIN_HEAP_MB: u64 = 512;

#[derive(Debug, Clone, Copy)]
pub struct SystemMemory {
    pub total_mb: u64,
    pub available_mb: u64,
}

impl SystemMemory {
    pub fn current() -> Self {
        let mut sys = System::new();
        sys.refresh_memory();
        Self {
            total_mb: sys.total_memory() / 1024 / 1024,
            available_mb: sys.available_memory() / 1024 / 1024,
        }
    }

    /// 8 GB or less: also use the lean JVM flags and the low-end options.txt.
    pub fn is_low(&self) -> bool {
        self.total_mb <= 8 * 1024
    }

    /// A quarter of RAM, between 768 MB and 3 GB. Vanilla rarely benefits from more,
    /// and a bigger heap only means longer GC pauses and less memory for everything else.
    pub fn recommended_max_heap_mb(&self) -> u64 {
        ((self.total_mb / 4) / 256 * 256).clamp(768, 3072)
    }
}

#[derive(Debug, Clone)]
pub struct HeapPlan {
    pub min_mb: u64,
    pub max_mb: u64,
    pub low_memory: bool,
    /// Set when the heap does not comfortably fit in currently free memory.
    pub warning: Option<String>,
}

/// Decide `-Xms`/`-Xmx`. Explicit values are respected (with a warning if they will not fit);
/// automatic values also shrink to what is actually free right now, so launching never pushes
/// the machine into swap.
pub fn plan_heap(
    mem: SystemMemory,
    explicit_max: Option<u32>,
    explicit_min: Option<u32>,
) -> HeapPlan {
    let fits = mem.available_mb.saturating_sub(NON_HEAP_MB);
    let mut warning = None;

    let max_mb = match explicit_max {
        Some(max) => {
            let max = max as u64;
            if max > fits {
                warning = Some(format!(
                    "max memory is {max} MB but only {} MB is free; the system may start swapping. \
                     Close other programs or lower --max-memory",
                    mem.available_mb
                ));
            }
            max
        }
        None => {
            let wanted = mem.recommended_max_heap_mb();
            if wanted <= fits {
                wanted
            } else {
                let shrunk = (fits / 128 * 128).max(MIN_HEAP_MB);
                warning = Some(format!(
                    "only {} MB free, so the game gets {shrunk} MB instead of {wanted} MB; \
                     close other programs for smoother play",
                    mem.available_mb
                ));
                shrunk
            }
        }
    };

    // A small -Xms means the heap grows only as needed instead of being claimed up front.
    let min_mb = explicit_min.map(|m| m as u64).unwrap_or(256).min(max_mb);

    HeapPlan {
        min_mb,
        max_mb,
        low_memory: mem.is_low() || max_mb <= 1536,
        warning,
    }
}

/// Parse a JVM size like `2G`, `512m` or `1048576k` into MB (rounded down).
pub fn parse_size_mb(size: &str) -> Option<u32> {
    let (digits, unit) = size.split_at(
        size.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(size.len()),
    );
    let n: u64 = digits.parse().ok()?;
    let mb = match unit.to_ascii_lowercase().as_str() {
        "" => n / (1024 * 1024),
        "k" => n / 1024,
        "m" => n,
        "g" => n * 1024,
        "t" => n * 1024 * 1024,
        _ => return None,
    };
    u32::try_from(mb).ok()
}

/// Mojang's suggested flags that we replace: heap size, GC choice and tuning, and
/// `AlwaysPreTouch`, which commits the whole heap at startup.
pub fn is_replaced_flag(flag: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "-Xmx",
        "-Xms",
        "-XX:+AlwaysPreTouch",
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:MaxGCPauseMillis",
        "-XX:G1",
        "-XX:+UseStringDeduplication",
        "-XX:+ZGenerational",
    ];
    PREFIXES.iter().any(|p| flag.starts_with(p)) || is_gc_selector(flag)
}

fn is_gc_selector(flag: &str) -> bool {
    flag.starts_with("-XX:+Use") && flag.ends_with("GC")
}

/// GC flags that work on every Java from 8 to 25 (an unknown flag would stop the JVM).
pub fn gc_flags(plan: &HeapPlan, user_args: &[String]) -> Vec<String> {
    if user_args.iter().any(|a| is_gc_selector(a)) {
        return Vec::new(); // The user picked a GC; two GC selectors make the JVM refuse to start.
    }
    let mut flags = vec![
        "-XX:+UseG1GC",
        "-XX:+ParallelRefProcEnabled",
        "-XX:+UseStringDeduplication",
    ];
    if !plan.low_memory {
        // Mojang's tuning: shorter pauses, but more headroom kept in reserve.
        flags.extend([
            "-XX:MaxGCPauseMillis=50",
            "-XX:+UnlockExperimentalVMOptions",
            "-XX:G1NewSizePercent=20",
            "-XX:G1ReservePercent=20",
        ]);
    }
    flags.into_iter().map(String::from).collect()
}

/// Lighter defaults for a new instance's `options.txt` on low-memory machines. Keys a version
/// does not know are ignored by the game.
pub const LOW_END_OPTIONS: &str = "\
renderDistance:6
simulationDistance:5
maxFps:60
particles:2
mipmapLevels:0
entityDistanceScaling:0.75
graphicsMode:0
fancyGraphics:false
renderClouds:false
";

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(total: u64, available: u64) -> SystemMemory {
        SystemMemory {
            total_mb: total,
            available_mb: available,
        }
    }

    #[test]
    fn recommended_heap_scales_with_ram() {
        assert_eq!(mem(2048, 0).recommended_max_heap_mb(), 768);
        assert_eq!(mem(4096, 0).recommended_max_heap_mb(), 1024);
        assert_eq!(mem(8192, 0).recommended_max_heap_mb(), 2048);
        assert_eq!(mem(32768, 0).recommended_max_heap_mb(), 3072);
    }

    #[test]
    fn auto_heap_shrinks_to_free_memory() {
        let plan = plan_heap(mem(8192, 2000), None, None);
        assert_eq!(plan.max_mb, 1152);
        assert!(plan.warning.is_some() && plan.low_memory);

        let plan = plan_heap(mem(16384, 12000), None, None);
        assert_eq!(plan.max_mb, 3072);
        assert!(plan.warning.is_none() && !plan.low_memory);
    }

    #[test]
    fn explicit_heap_is_kept_but_warned() {
        let plan = plan_heap(mem(4096, 1500), Some(4096), None);
        assert_eq!(plan.max_mb, 4096);
        assert!(plan.warning.is_some());
    }

    #[test]
    fn parses_jvm_sizes() {
        assert_eq!(parse_size_mb("2G"), Some(2048));
        assert_eq!(parse_size_mb("512m"), Some(512));
        assert_eq!(parse_size_mb("1048576k"), Some(1024));
        assert_eq!(parse_size_mb("abc"), None);
        assert_eq!(parse_size_mb("99999999999M"), None); // doesn't fit; JVM will reject it anyway
    }

    #[test]
    fn replaces_mojang_flags() {
        for f in [
            "-Xmx4G",
            "-XX:+AlwaysPreTouch",
            "-XX:+UseZGC",
            "-XX:G1NewSizePercent=20",
        ] {
            assert!(is_replaced_flag(f), "{f}");
        }
        assert!(!is_replaced_flag("-XX:+UseCompactObjectHeaders"));
    }

    #[test]
    fn user_gc_choice_wins() {
        let plan = plan_heap(mem(16384, 12000), None, None);
        assert!(gc_flags(&plan, &["-XX:+UseZGC".into()]).is_empty());
        assert!(gc_flags(&plan, &[]).contains(&"-XX:+UseG1GC".to_string()));
    }
}
