//! Telling mods apart by what is inside them: a mod's jar says its own id, name and version.
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::delete::pick_instance;

pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

fn read_entry(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Option<String> {
    let mut text = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut text).ok()?;
    Some(text)
}

/// `Implementation-Version` from the jar's manifest, which Forge-style mods point their `${file.jarVersion}` at.
fn manifest_version(zip: &mut zip::ZipArchive<fs::File>) -> Option<String> {
    read_entry(zip, "META-INF/MANIFEST.MF")?
        .lines()
        .find_map(|l| l.strip_prefix("Implementation-Version:"))
        .map(|v| v.trim().to_string())
}

/// Reads Fabric, Quilt, NeoForge or Forge metadata; None if the jar has none of them.
pub fn read_info(jar: &Path) -> Option<ModInfo> {
    let mut zip = zip::ZipArchive::new(fs::File::open(jar).ok()?).ok()?;
    let text = |v: &Value| v.as_str().map(str::to_string);

    if let Some(json) = read_entry(&mut zip, "fabric.mod.json") {
        // Some mods have stray characters in this file, so be forgiving about what we need
        let v: Value = serde_json::from_str(&json).ok()?;
        let id = text(&v["id"])?;
        return Some(ModInfo { name: text(&v["name"]).unwrap_or_else(|| id.clone()), version: text(&v["version"]).unwrap_or_default(), id });
    }
    if let Some(json) = read_entry(&mut zip, "quilt.mod.json") {
        let v: Value = serde_json::from_str(&json).ok()?;
        let loader = &v["quilt_loader"];
        let id = text(&loader["id"])?;
        return Some(ModInfo {
            name: text(&loader["metadata"]["name"]).unwrap_or_else(|| id.clone()),
            version: text(&loader["version"]).unwrap_or_default(),
            id,
        });
    }
    for file in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        let Some(toml_text) = read_entry(&mut zip, file) else { continue };
        let v: toml::Value = toml::from_str(&toml_text).ok()?;
        let first = v.get("mods")?.as_array()?.first()?;
        let get = |key: &str| first.get(key).and_then(|x| x.as_str()).map(str::to_string);
        let id = get("modId")?;
        let version = match get("version") {
            Some(v) if v.contains("${") => manifest_version(&mut zip).unwrap_or_default(),
            Some(v) => v,
            None => String::new(),
        };
        return Some(ModInfo { name: get("displayName").unwrap_or_else(|| id.clone()), version, id });
    }
    None
}

fn jars(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar")))
        .collect();
    found.sort();
    found
}

pub fn list_mods(instance: Option<&str>) -> Result<()> {
    let dir = pick_instance(instance)?.join("game").join("mods");
    let mods = jars(&dir);
    if mods.is_empty() {
        say!("no mods in {}", dir.display());
    }
    for jar in mods {
        let file = jar.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match read_info(&jar) {
            Some(m) => say!("{:<24} {:<28} {:<14} {file}", m.id, m.name, m.version),
            None => say!("{:<24} {:<28} {:<14} {file}", "?", "(no mod info found)", ""),
        }
    }
    Ok(())
}
