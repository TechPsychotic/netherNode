//! netherNode core: an offline-only Minecraft launcher library.
//!
//! Game files come from Mojang's public download servers; accounts are purely local.

pub mod account;
pub mod crash;
pub mod download;
pub mod error;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod loader;
pub mod log;
pub mod memory;
pub mod modpack;
pub mod mods;
pub mod paths;
pub mod rules;
pub mod version;

pub use error::{Error, Result};
