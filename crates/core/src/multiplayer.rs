//! Minecraft's server list (`servers.dat` in the game folder), so servers hosted on this PC
//! show up under Multiplayer like any other server.

use std::collections::HashMap;
use std::path::Path;

use fastnbt::Value;

use crate::error::{Result, other};

fn load(file: &Path) -> Vec<Value> {
    let Ok(bytes) = std::fs::read(file) else {
        return Vec::new();
    };
    match fastnbt::from_bytes::<Value>(&bytes) {
        Ok(Value::Compound(mut root)) => match root.remove("servers") {
            Some(Value::List(list)) => list,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn save(file: &Path, servers: Vec<Value>) -> Result<()> {
    let mut root = HashMap::new();
    root.insert("servers".to_string(), Value::List(servers));
    let bytes = fastnbt::to_bytes(&Value::Compound(root))
        .map_err(|e| other(format!("couldn't write the server list: {e}")))?;
    std::fs::write(file, bytes)?;
    Ok(())
}

fn string_field<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    match entry {
        Value::Compound(map) => match map.get(key) {
            Some(Value::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

/// Add (or rename) a server in the Multiplayer list of the game in `game_dir`, matched by
/// address. Other entries are left as they are. Takes effect next time the list is opened;
/// a running game may overwrite it when its Multiplayer screen closes.
pub fn add(game_dir: &Path, name: &str, address: &str) -> Result<()> {
    let file = game_dir.join("servers.dat");
    let mut servers = load(&file);
    let entry = |name: &str| {
        let mut map = HashMap::new();
        map.insert("name".to_string(), Value::String(name.to_string()));
        map.insert("ip".to_string(), Value::String(address.to_string()));
        Value::Compound(map)
    };
    match servers
        .iter_mut()
        .find(|s| string_field(s, "ip") == Some(address))
    {
        Some(Value::Compound(map)) => {
            map.insert("name".to_string(), Value::String(name.to_string()));
        }
        _ => servers.insert(0, entry(name)),
    }
    std::fs::create_dir_all(game_dir)?;
    save(&file, servers)
}

/// Remove the server with this address from the list, if it's there.
pub fn remove(game_dir: &Path, address: &str) -> Result<()> {
    let file = game_dir.join("servers.dat");
    if !file.exists() {
        return Ok(());
    }
    let mut servers = load(&file);
    let before = servers.len();
    servers.retain(|s| string_field(s, "ip") != Some(address));
    if servers.len() != before {
        save(&file, servers)?;
    }
    Ok(())
}

/// Names and addresses in the list, in order.
pub fn list(game_dir: &Path) -> Vec<(String, String)> {
    load(&game_dir.join("servers.dat"))
        .iter()
        .filter_map(|s| {
            Some((
                string_field(s, "name")?.to_string(),
                string_field(s, "ip")?.to_string(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_updates_and_removes_entries() {
        let dir = std::env::temp_dir().join(format!("nethernode-mp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        add(&dir, "Old name", "localhost:25565").unwrap();
        add(&dir, "Hypixel", "mc.hypixel.net").unwrap();
        add(&dir, "Survival (this PC)", "localhost:25565").unwrap();
        assert_eq!(
            list(&dir),
            [
                ("Hypixel".to_string(), "mc.hypixel.net".to_string()),
                (
                    "Survival (this PC)".to_string(),
                    "localhost:25565".to_string()
                ),
            ]
        );
        remove(&dir, "localhost:25565").unwrap();
        assert_eq!(list(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
