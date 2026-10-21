//! Offline accounts. There is no network authentication of any kind: an account is a name
//! plus the UUID vanilla servers derive for that name in offline mode.

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use crate::error::{Result, other};
use crate::paths::{Paths, ensure_parent};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub uuid: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Accounts {
    pub default: Option<String>,
    pub accounts: Vec<Account>,
}

/// Same result as Java's `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`,
/// which is what servers in offline mode and LAN worlds use.
pub fn offline_uuid(name: &str) -> String {
    let mut bytes: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30; // version 3
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // IETF variant
    let h = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

pub fn validate_name(name: &str) -> Result<()> {
    let ok_len = (3..=16).contains(&name.len());
    let ok_chars = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok_len && ok_chars {
        Ok(())
    } else {
        Err(other(
            "username must be 3-16 characters: letters, digits or _",
        ))
    }
}

impl Accounts {
    pub fn load(paths: &Paths) -> Result<Self> {
        match std::fs::read(paths.accounts_file()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = paths.accounts_file();
        ensure_parent(&file)?;
        std::fs::write(file, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    fn find(&self, name: &str) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Add an offline account. The first account added becomes the default.
    pub fn add(&mut self, name: &str) -> Result<Account> {
        validate_name(name)?;
        if self.find(name).is_some() {
            return Err(other(format!("account '{name}' already exists")));
        }
        let account = Account {
            name: name.to_string(),
            uuid: offline_uuid(name),
        };
        self.accounts.push(account.clone());
        if self.default.is_none() {
            self.default = Some(account.name.clone());
        }
        Ok(account)
    }

    pub fn remove(&mut self, name: &str) -> Result<()> {
        let before = self.accounts.len();
        self.accounts.retain(|a| !a.name.eq_ignore_ascii_case(name));
        if self.accounts.len() == before {
            return Err(other(format!("no account named '{name}'")));
        }
        if self
            .default
            .as_deref()
            .is_some_and(|d| d.eq_ignore_ascii_case(name))
        {
            self.default = self.accounts.first().map(|a| a.name.clone());
        }
        Ok(())
    }

    pub fn set_default(&mut self, name: &str) -> Result<()> {
        let account = self
            .find(name)
            .ok_or_else(|| other(format!("no account named '{name}'")))?;
        self.default = Some(account.name.clone());
        Ok(())
    }

    /// The named account, or the default one when `name` is `None`.
    pub fn pick(&self, name: Option<&str>) -> Result<Account> {
        let name = name
            .or(self.default.as_deref())
            .ok_or_else(|| other("no account; add one with `nethernode account add <name>`"))?;
        self.find(name)
            .cloned()
            .ok_or_else(|| other(format!("no account named '{name}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_name_uuid_from_bytes() {
        // Reference values computed with java.util.UUID.nameUUIDFromBytes.
        assert_eq!(
            offline_uuid("Notch"),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
        assert_eq!(
            offline_uuid("Steve"),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
        assert_eq!(offline_uuid("jeb_"), "a762f560-4fce-3236-812a-b80efff0b62b");
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("Steve_123").is_ok());
        assert!(validate_name("ab").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("waytoolongusername").is_err());
    }

    #[test]
    fn first_account_is_default() {
        let mut a = Accounts::default();
        a.add("Alex").unwrap();
        a.add("Steve").unwrap();
        assert_eq!(a.pick(None).unwrap().name, "Alex");
        a.remove("alex").unwrap();
        assert_eq!(a.pick(None).unwrap().name, "Steve");
        assert!(a.add("steve").is_err());
    }
}
