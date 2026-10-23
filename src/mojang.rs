//! Talking to Mojang's version manifest.
use anyhow::{Context, Result};
use serde::Deserialize;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Deserialize)]
pub struct Manifest {
    pub latest: Latest,
    pub versions: Vec<Version>,
}

#[derive(Deserialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Deserialize)]
pub struct Version {
    pub id: String,
    /// "release", "snapshot", "old_beta" or "old_alpha"
    #[serde(rename = "type")]
    pub kind: String,
    /// Where this version's own description (libraries, assets, ...) lives
    pub url: String,
    pub sha1: String,
}

pub fn fetch_manifest() -> Result<Manifest> {
    reqwest::blocking::get(MANIFEST_URL)
        .and_then(|r| r.error_for_status())
        .context("could not download Mojang's version manifest")?
        .json()
        .context("Mojang's version manifest was not what we expected")
}

pub fn find_version<'a>(manifest: &'a Manifest, id: &str) -> Result<&'a Version> {
    manifest
        .versions
        .iter()
        .find(|v| v.id == id)
        .with_context(|| format!("Minecraft has no version \"{id}\" (see `mcterm list versions --snapshots --beta --pre-alpha`)"))
}
