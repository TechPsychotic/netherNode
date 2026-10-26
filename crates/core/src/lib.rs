//! netherNode core: an offline-only Minecraft launcher library.
//!
//! Game files come from Mojang's public download servers; accounts are purely local.

pub mod account;
pub mod crash;
pub mod download;
pub mod error;
pub mod friends;
pub mod install;
pub mod instance;
pub mod invite;
pub mod java;
pub mod launch;
pub mod loader;
pub mod log;
pub mod memory;
pub mod modpack;
pub mod mods;
pub mod modsync;
pub mod multiplayer;
pub mod paths;
pub mod rules;
pub mod server;
pub mod settings;
pub mod share;
pub mod skins;
pub mod version;
pub mod worlds;

pub use error::{Error, Result};
