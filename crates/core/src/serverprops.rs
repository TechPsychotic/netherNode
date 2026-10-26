//! What each `server.properties` setting means, so the app can show proper controls (a tick
//! box, choices or a number) with an explanation instead of raw text. Settings the server
//! has that aren't listed here (newer versions, Paper, mods) are still editable as text.

/// How a setting is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// A whole number within limits.
    Number {
        min: i64,
        max: i64,
    },
    /// One of these values.
    Choice(&'static [&'static str]),
    Text,
}

#[derive(Debug, Clone, Copy)]
pub struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    /// Minecraft's default, shown before the server first writes its file.
    pub default: &'static str,
    pub group: Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    General,
    Gameplay,
    World,
    Access,
    Network,
    Advanced,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::General,
        Group::Gameplay,
        Group::World,
        Group::Access,
        Group::Network,
        Group::Advanced,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::General => "General",
            Group::Gameplay => "Gameplay",
            Group::World => "World",
            Group::Access => "Players and access",
            Group::Network => "Network and performance",
            Group::Advanced => "Advanced",
        }
    }
}

/// Settings netherNode manages itself: the port it picked, online mode (from the skins
/// option), chat signing (offline accounts can't sign) and the whitelist (its own editor).
pub const MANAGED: &[&str] = &[
    "server-port",
    "online-mode",
    "enforce-secure-profile",
    "white-list",
    "enforce-whitelist",
];

const fn s(
    key: &'static str,
    label: &'static str,
    help: &'static str,
    kind: Kind,
    default: &'static str,
    group: Group,
) -> Setting {
    Setting {
        key,
        label,
        help,
        kind,
        default,
        group,
    }
}

const BOOL: Kind = Kind::Bool;
const TEXT: Kind = Kind::Text;
const fn num(min: i64, max: i64) -> Kind {
    Kind::Number { min, max }
}

pub const SETTINGS: &[Setting] = &[
    s(
        "motd",
        "Message in the server list",
        "Shown under the server's name in Minecraft's Multiplayer list.",
        TEXT,
        "A Minecraft Server",
        Group::General,
    ),
    s(
        "max-players",
        "Max players",
        "How many players can be online at once.",
        num(1, 1000),
        "20",
        Group::General,
    ),
    s(
        "gamemode",
        "Game mode",
        "The mode new players start in.",
        Kind::Choice(&["survival", "creative", "adventure", "spectator"]),
        "survival",
        Group::Gameplay,
    ),
    s(
        "force-gamemode",
        "Force game mode",
        "Put players back in the game mode above every time they join.",
        BOOL,
        "false",
        Group::Gameplay,
    ),
    s(
        "difficulty",
        "Difficulty",
        "How hard mobs hit and how hungry players get.",
        Kind::Choice(&["peaceful", "easy", "normal", "hard"]),
        "easy",
        Group::Gameplay,
    ),
    s(
        "hardcore",
        "Hardcore",
        "One life: players who die become spectators. Difficulty is locked to hard.",
        BOOL,
        "false",
        Group::Gameplay,
    ),
    s(
        "pvp",
        "PvP",
        "Players can hurt each other.",
        BOOL,
        "true",
        Group::Gameplay,
    ),
    s(
        "allow-flight",
        "Allow flight",
        "Don't kick players for flying (needed by some mods and plugins).",
        BOOL,
        "false",
        Group::Gameplay,
    ),
    s(
        "allow-nether",
        "Allow the Nether",
        "Players can travel to the Nether.",
        BOOL,
        "true",
        Group::Gameplay,
    ),
    s(
        "spawn-monsters",
        "Monsters",
        "Hostile mobs spawn.",
        BOOL,
        "true",
        Group::Gameplay,
    ),
    s(
        "enable-command-block",
        "Command blocks",
        "Command blocks work.",
        BOOL,
        "false",
        Group::Gameplay,
    ),
    s(
        "spawn-protection",
        "Spawn protection",
        "Blocks around spawn (radius) that only operators can change. 0 turns it off.",
        num(0, 1000),
        "16",
        Group::Gameplay,
    ),
    s(
        "level-name",
        "World folder",
        "The world's folder name. Changing it starts a different world.",
        TEXT,
        "world",
        Group::World,
    ),
    s(
        "level-seed",
        "Seed",
        "Used only when a new world is created. Empty means random.",
        TEXT,
        "",
        Group::World,
    ),
    s(
        "level-type",
        "World type",
        "For new worlds: minecraft:normal, minecraft:flat, minecraft:large_biomes or minecraft:amplified (older versions: default, flat, largeBiomes, amplified).",
        TEXT,
        "minecraft:normal",
        Group::World,
    ),
    s(
        "generate-structures",
        "Structures",
        "Villages, temples and other structures generate in new chunks.",
        BOOL,
        "true",
        Group::World,
    ),
    s(
        "view-distance",
        "View distance",
        "How many chunks around each player are sent (2–32). Lower is lighter on this PC.",
        num(2, 32),
        "10",
        Group::World,
    ),
    s(
        "simulation-distance",
        "Simulation distance",
        "How many chunks around each player keep running (2–32). Lower is lighter on this PC.",
        num(2, 32),
        "10",
        Group::World,
    ),
    s(
        "max-world-size",
        "World border radius",
        "How far the world goes, in blocks from the centre.",
        num(1, 29_999_984),
        "29999984",
        Group::World,
    ),
    s(
        "player-idle-timeout",
        "Kick idle players after (minutes)",
        "0 never kicks.",
        num(0, 100_000),
        "0",
        Group::Access,
    ),
    s(
        "op-permission-level",
        "Operator level",
        "What operators may do: 1 build at spawn, 2 cheats, 3 kick/ban, 4 everything.",
        num(1, 4),
        "4",
        Group::Access,
    ),
    s(
        "hide-online-players",
        "Hide who's online",
        "Don't show the player list in the server list.",
        BOOL,
        "false",
        Group::Access,
    ),
    s(
        "enable-status",
        "Show in the server list",
        "Answer server-list pings (off makes it look offline).",
        BOOL,
        "true",
        Group::Access,
    ),
    s(
        "resource-pack",
        "Resource pack URL",
        "A direct download link to a resource pack players get when joining.",
        TEXT,
        "",
        Group::Access,
    ),
    s(
        "require-resource-pack",
        "Require the resource pack",
        "Players who decline the pack are disconnected.",
        BOOL,
        "false",
        Group::Access,
    ),
    s(
        "network-compression-threshold",
        "Compression threshold",
        "Packets larger than this (bytes) are compressed. -1 turns it off.",
        num(-1, 65_536),
        "256",
        Group::Network,
    ),
    s(
        "rate-limit",
        "Packet rate limit",
        "Kick players sending more packets per second than this. 0 turns it off.",
        num(0, 100_000),
        "0",
        Group::Network,
    ),
    s(
        "prevent-proxy-connections",
        "Block VPN/proxy logins",
        "Only works for online servers; leave off.",
        BOOL,
        "false",
        Group::Network,
    ),
    s(
        "sync-chunk-writes",
        "Safe chunk saving",
        "Write chunks to disk immediately. Safer, slower on slow disks.",
        BOOL,
        "true",
        Group::Network,
    ),
    s(
        "entity-broadcast-range-percentage",
        "Entity view range (%)",
        "How far away players see entities, as a percentage of normal.",
        num(10, 1000),
        "100",
        Group::Network,
    ),
    s(
        "max-tick-time",
        "Watchdog (ms)",
        "Stop the server if one tick takes longer than this. -1 turns it off (useful for big modpacks).",
        num(-1, 1_000_000_000),
        "60000",
        Group::Network,
    ),
    s(
        "enable-rcon",
        "Remote console (RCON)",
        "Allow remote admin tools to connect.",
        BOOL,
        "false",
        Group::Advanced,
    ),
    s(
        "rcon.port",
        "RCON port",
        "",
        num(1, 65_535),
        "25575",
        Group::Advanced,
    ),
    s(
        "rcon.password",
        "RCON password",
        "",
        TEXT,
        "",
        Group::Advanced,
    ),
    s(
        "enable-query",
        "Query protocol",
        "Answer GameSpy4 queries (server-list websites).",
        BOOL,
        "false",
        Group::Advanced,
    ),
    s(
        "query.port",
        "Query port",
        "",
        num(1, 65_535),
        "25565",
        Group::Advanced,
    ),
    s(
        "broadcast-console-to-ops",
        "Send console output to operators",
        "",
        BOOL,
        "true",
        Group::Advanced,
    ),
    s(
        "function-permission-level",
        "Function permission level",
        "Permission level for datapack functions (1–4).",
        num(1, 4),
        "2",
        Group::Advanced,
    ),
];

/// The catalog entry for `key`, if known.
pub fn find(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// Check a value typed for `key`. Returns the value to store (trimmed, normalised).
pub fn validate(key: &str, value: &str) -> std::result::Result<String, String> {
    let value = value.trim();
    let Some(setting) = find(key) else {
        return Ok(value.to_string());
    };
    match setting.kind {
        Kind::Bool => match value {
            "true" | "false" => Ok(value.to_string()),
            _ => Err(format!("{} must be on or off", setting.label)),
        },
        Kind::Number { min, max } => match value.parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(n.to_string()),
            _ => Err(format!(
                "{} must be a number from {min} to {max}",
                setting.label
            )),
        },
        Kind::Choice(choices) => {
            if choices.contains(&value) {
                Ok(value.to_string())
            } else {
                Err(format!(
                    "{} must be one of: {}",
                    setting.label,
                    choices.join(", ")
                ))
            }
        }
        Kind::Text => Ok(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent_and_validates() {
        for setting in SETTINGS {
            assert!(
                validate(setting.key, setting.default).is_ok(),
                "{}",
                setting.key
            );
            assert!(!MANAGED.contains(&setting.key), "{}", setting.key);
        }
        assert_eq!(validate("view-distance", " 12 ").unwrap(), "12");
        assert!(validate("view-distance", "64").is_err());
        assert!(validate("pvp", "yes").is_err());
        assert!(validate("difficulty", "nightmare").is_err());
        assert_eq!(
            validate("some-paper-setting", "anything").unwrap(),
            "anything"
        );
    }
}
