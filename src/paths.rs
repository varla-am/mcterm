//! Where mcterm keeps its files: `$MCTERM_HOME`, or `~/.mcterm`.
//!
//!   versions/<id>/<id>.json, <id>.jar, natives/   what Mojang describes for a version
//!   libraries/                                    shared by every version
//!   assets/indexes, assets/objects                shared sounds, languages, ...
//!   instances/<name>/instance.json, game/         one folder per instance
use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MCTERM_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").context("$HOME is not set")?;
    Ok(PathBuf::from(home).join(".mcterm"))
}

pub fn versions() -> Result<PathBuf> {
    Ok(root()?.join("versions"))
}

pub fn libraries() -> Result<PathBuf> {
    Ok(root()?.join("libraries"))
}

pub fn assets() -> Result<PathBuf> {
    Ok(root()?.join("assets"))
}

pub fn instances() -> Result<PathBuf> {
    Ok(root()?.join("instances"))
}
