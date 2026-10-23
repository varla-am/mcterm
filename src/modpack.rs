//! `mcterm install modpack`: a Modrinth `.mrpack` is a zip with an index of mods to download,
//! plus an `overrides/` folder of files (configs, ...) to copy into the game folder.
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::download::{download_all, Job};
use crate::install;
use crate::paths;

/// The only places the `.mrpack` format lets a launcher download from
const ALLOWED_HOSTS: [&str; 4] = ["cdn.modrinth.com", "github.com", "raw.githubusercontent.com", "gitlab.com"];

#[derive(Deserialize)]
struct Index {
    name: String,
    #[serde(default)]
    files: Vec<PackFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Deserialize)]
struct PackFile {
    path: String,
    hashes: HashMap<String, String>,
    #[serde(default)]
    downloads: Vec<String>,
    env: Option<Env>,
}

#[derive(Deserialize)]
struct Env {
    client: Option<String>,
}

/// A path from inside the pack, only if it stays inside the game folder.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    path.components().all(|c| matches!(c, Component::Normal(_))).then(|| path.to_path_buf())
}

fn host_allowed(url: &str) -> bool {
    url.strip_prefix("https://").and_then(|rest| rest.split('/').next()).is_some_and(|host| ALLOWED_HOSTS.contains(&host))
}

pub fn install_modpack(pack: &Path, name: Option<&str>, loader_version: Option<&str>) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(pack).with_context(|| format!("could not open {}", pack.display()))?)
        .context("that is not a .mrpack (a zip file)")?;
    let mut text = String::new();
    zip.by_name("modrinth.index.json")
        .context("no modrinth.index.json inside: only Modrinth packs (.mrpack) are supported")?
        .read_to_string(&mut text)?;
    let index: Index = serde_json::from_str(&text).context("modrinth.index.json is not what we expected")?;

    // What the pack runs on
    let mc = index.dependencies.get("minecraft").context("the pack does not say which Minecraft version it needs")?;
    let loaders = [("fabric-loader", "fabric"), ("quilt-loader", "quilt"), ("neoforge", "neoforge"), ("forge", "forge")];
    let found: Vec<(&str, &String)> = loaders.iter().filter_map(|(key, loader)| index.dependencies.get(*key).map(|v| (*loader, v))).collect();
    if found.len() > 1 {
        bail!("the pack asks for more than one modloader");
    }
    let (loader, pack_loader_version) = found.first().map_or((None, None), |(l, v)| (Some(*l), Some(v.as_str())));
    let name = name.map(str::to_string).unwrap_or_else(|| index.name.replace(['/', '\\', ':'], "-"));
    say!("modpack \"{}\": Minecraft {mc}{}", index.name, loader.map(|l| format!(" with {l}")).unwrap_or_default());

    // Everything the game needs, as for any instance
    install::install_instance(mc, loader, loader_version.or(pack_loader_version), Some(&name))?;
    let game_dir = paths::instances()?.join(&name).join("game");

    // The mods and other files the pack lists
    let mut jobs = Vec::new();
    for file in &index.files {
        if file.env.as_ref().and_then(|e| e.client.as_deref()) == Some("unsupported") {
            continue;
        }
        let rel = safe_relative(&file.path).with_context(|| format!("the pack puts a file outside the game folder: {}", file.path))?;
        let url = file
            .downloads
            .iter()
            .find(|u| host_allowed(u))
            .with_context(|| format!("{} has no download from a site the format allows", file.path))?;
        let sha1 = file.hashes.get("sha1").with_context(|| format!("{} has no sha1", file.path))?;
        jobs.push(Job { url: url.clone(), dest: game_dir.join(rel), sha1: Some(sha1.clone()) });
    }
    download_all("mods", jobs)?;

    // Configs and the like; client-overrides win over overrides
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let Ok(entry_name) = entry.name().map(|n| n.into_owned()) else { continue };
            let Some(rest) = entry_name.strip_prefix(prefix).map(str::to_string) else { continue };
            let Some(rel) = safe_relative(&rest) else { continue };
            if entry.is_dir() || rest.is_empty() {
                continue;
            }
            let dest = game_dir.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            io::copy(&mut entry, &mut fs::File::create(&dest)?)?;
        }
    }

    say!("modpack ready: mcterm launch instance \"{name}\"");
    Ok(())
}
