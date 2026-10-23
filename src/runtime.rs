//! Java: the runtime Mojang ships for each Minecraft version, downloaded once into `runtimes/`.
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::download::{download_all, Job};
use crate::paths;

const INDEX_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Returns the `java` executable for a runtime component like "java-runtime-delta", installing it if needed.
pub fn ensure_java(component: &str) -> Result<PathBuf> {
    let dir = paths::root()?.join("runtimes").join(component);
    let java = dir.join("jre.bundle/Contents/Home/bin/java");
    if java.exists() {
        return Ok(java);
    }

    let index: Value = reqwest::blocking::get(INDEX_URL)?.error_for_status()?.json()?;
    // Apple Silicon first; Mojang has no native build of the older runtimes, so those run as Intel under Rosetta
    let platforms: &[&str] = if std::env::consts::ARCH == "aarch64" { &["mac-os-arm64", "mac-os"] } else { &["mac-os"] };
    let manifest_url = platforms
        .iter()
        .find_map(|p| index[*p][component][0]["manifest"]["url"].as_str())
        .with_context(|| format!("Mojang has no Java runtime \"{component}\" for this Mac"))?;

    let manifest: Value = reqwest::blocking::get(manifest_url)?.error_for_status()?.json()?;
    let files = manifest["files"].as_object().context("the Java runtime manifest has no files")?;

    let mut jobs = Vec::new();
    let mut executables = Vec::new();
    let mut links = Vec::new();
    for (path, entry) in files {
        let dest = dir.join(path);
        match entry["type"].as_str() {
            Some("directory") => fs::create_dir_all(&dest)?,
            Some("link") => links.push((dest, entry["target"].as_str().unwrap_or_default().to_string())),
            Some("file") => {
                let raw = &entry["downloads"]["raw"];
                jobs.push(Job {
                    url: raw["url"].as_str().context("a runtime file has no url")?.to_string(),
                    dest: dest.clone(),
                    sha1: raw["sha1"].as_str().map(str::to_string),
                });
                if entry["executable"].as_bool() == Some(true) {
                    executables.push(dest);
                }
            }
            _ => {}
        }
    }
    download_all(&format!("Java ({component})"), jobs)?;
    for path in executables {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    for (link, target) in links {
        if let Some(parent) = link.parent() {
            fs::create_dir_all(parent)?;
        }
        let _ = fs::remove_file(&link);
        symlink(&target, &link)?;
    }

    if !java.exists() {
        bail!("installed the Java runtime, but {} is missing", java.display());
    }
    Ok(java)
}
