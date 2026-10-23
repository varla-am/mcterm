//! `mcterm install instance`: fetch everything a version needs, then make an instance for it.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::download::{download_all, download_one, Job};
use crate::loader;
use crate::mojang;
use crate::paths;
use crate::version::Artifact;

const ASSET_URL: &str = "https://resources.download.minecraft.net";

#[derive(Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub version: String,
    pub modloader: Option<String>,
    /// The version id to launch: the loader's profile, which builds on `version`
    #[serde(default)]
    pub profile: Option<String>,
    /// Which Java the game needs (8 for old versions)
    pub java_major: u32,
}

pub fn load_instance(dir: &Path) -> Result<Instance> {
    let path = dir.join("instance.json");
    serde_json::from_slice(&fs::read(&path)?).with_context(|| format!("{} is damaged", path.display()))
}

#[derive(Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
}

pub fn install_instance(version: &str, modloader: Option<&str>, loader_version: Option<&str>, name: Option<&str>) -> Result<()> {
    let default_name = match modloader {
        Some(loader) => format!("{version}-{loader}"),
        None => version.to_string(),
    };
    if let Some(loader) = modloader {
        if !loader::LOADERS.split(", ").any(|l| l == loader) {
            bail!("unknown modloader \"{loader}\" (try one of: {})", loader::LOADERS);
        }
    }
    let name = name.unwrap_or(&default_name);
    let instance_dir = paths::instances()?.join(name);
    if instance_dir.exists() {
        bail!("instance \"{name}\" already exists");
    }

    let manifest = mojang::fetch_manifest()?;
    let entry = mojang::find_version(&manifest, version)?;

    // The version's own description
    let version_dir = paths::versions()?.join(version);
    let json_path = version_dir.join(format!("{version}.json"));
    download_one(&entry.url, &json_path, Some(&entry.sha1))?;
    let vanilla = crate::version::load(version)?;
    let java_component = vanilla.java_version.as_ref().map_or("jre-legacy", |j| j.component.as_str());

    // The modloader lays its profile over vanilla; from here on, `v` is the whole thing
    let profile = match modloader {
        Some(loader) => Some(loader::install(loader, version, loader_version, java_component)?),
        None => None,
    };
    let v = crate::version::load(profile.as_deref().unwrap_or(version))?;

    // The game jar, libraries, and the log settings
    let libraries_dir = paths::libraries()?;
    let mut jobs = vec![Job {
        url: v.downloads.client.url.clone(),
        dest: version_dir.join(format!("{version}.jar")),
        sha1: Some(v.downloads.client.sha1.clone()),
    }];
    if let Some(file) = v.logging.as_ref().and_then(|l| l.client.as_ref()).map(|c| &c.file) {
        jobs.push(Job {
            url: file.url.clone(),
            dest: paths::assets()?.join("log_configs").join(&file.id),
            sha1: Some(file.sha1.clone()),
        });
    }
    let mut seen = HashSet::new();
    let mut natives = Vec::new();
    for lib in v.libraries.iter().filter(|l| l.applies()) {
        for artifact in lib.artifact().into_iter().chain(lib.native_file().and_then(|f| {
            Some(Artifact { path: f.path.clone()?, url: f.url.clone(), sha1: Some(f.sha1.clone()) })
        })) {
            // No url: a jar the modloader's installer builds itself
            if !artifact.url.is_empty() && seen.insert(artifact.path.clone()) {
                jobs.push(Job { url: artifact.url, dest: libraries_dir.join(&artifact.path), sha1: artifact.sha1 });
            }
        }
        if let Some(path) = lib.native_file().and_then(|f| f.path.as_ref()) {
            let exclude = lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default();
            natives.push((libraries_dir.join(path), exclude));
        }
    }
    download_all("game and libraries", jobs)?;

    // Old-style natives are zips that have to be unpacked next to the game
    let natives_dir = version_dir.join("natives");
    for (jar, exclude) in &natives {
        extract_natives(jar, &natives_dir, exclude)?;
    }

    // Sounds, languages, ...: an index of names, then one file per hash
    let assets_dir = paths::assets()?;
    let index_path = assets_dir.join("indexes").join(format!("{}.json", v.asset_index.id));
    download_one(&v.asset_index.url, &index_path, Some(&v.asset_index.sha1))?;
    let index: AssetIndex = serde_json::from_slice(&fs::read(&index_path)?)?;
    let hashes: HashSet<&String> = index.objects.values().map(|o| &o.hash).collect();
    let asset_jobs = hashes
        .into_iter()
        .map(|hash| Job {
            url: format!("{ASSET_URL}/{}/{hash}", &hash[..2]),
            dest: assets_dir.join("objects").join(&hash[..2]).join(hash),
            sha1: Some(hash.clone()),
        })
        .collect();
    download_all("assets", asset_jobs)?;

    // The instance itself
    let java_major = v.java_version.as_ref().map_or(8, |j| j.major);
    fs::create_dir_all(instance_dir.join("game"))?;
    let instance = Instance {
        name: name.to_string(),
        version: version.to_string(),
        modloader: modloader.map(str::to_string),
        profile,
        java_major,
    };
    fs::write(instance_dir.join("instance.json"), serde_json::to_string_pretty(&instance)?)?;

    say!("Installed {version}{} as \"{name}\" (needs Java {java_major})", modloader.map(|m| format!(" with {m}")).unwrap_or_default());
    Ok(())
}

fn extract_natives(jar: &Path, dest: &Path, exclude: &[String]) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(jar)?)?;
    fs::create_dir_all(dest)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if entry.is_dir() || exclude.iter().any(|ex| rel.starts_with(ex)) {
            continue;
        }
        let Some(file_name) = rel.file_name() else { continue };
        io::copy(&mut entry, &mut fs::File::create(dest.join(file_name))?)?;
    }
    Ok(())
}

pub fn list_instances() -> Result<()> {
    let dir = paths::instances()?;
    let mut found = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path().join("instance.json");
            if let Ok(bytes) = fs::read(&path) {
                found.push(serde_json::from_slice::<Instance>(&bytes)?);
            }
        }
    }
    if found.is_empty() {
        say!("No instances yet. Try `mcterm install instance <version>`.");
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    for i in found {
        say!("{:<24} {}{}", i.name, i.version, i.modloader.map(|m| format!(" ({m})")).unwrap_or_default());
    }
    Ok(())
}
