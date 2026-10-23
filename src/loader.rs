//! Modloaders: Fabric and Quilt hand out a ready profile; Forge and NeoForge ship an installer to run.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::download::download_one;
use crate::paths;
use crate::runtime;

pub const LOADERS: &str = "fabric, quilt, neoforge, forge";

fn get_json(url: &str) -> Result<Value> {
    reqwest::blocking::get(url)
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("could not reach {url}"))?
        .json()
        .with_context(|| format!("{url} did not answer with JSON"))
}

/// Installs the loader for a Minecraft version (whose vanilla json is already in `versions/`),
/// and returns the id of the profile to launch.
/// `java_component` is the Java used to run installers.
pub fn install(loader: &str, mc: &str, loader_version: Option<&str>, java_component: &str) -> Result<String> {
    match loader {
        "fabric" => meta_profile("Fabric", "https://meta.fabricmc.net/v2", mc, loader_version),
        "quilt" => meta_profile("Quilt", "https://meta.quiltmc.org/v3", mc, loader_version),
        "neoforge" => {
            let version = match loader_version {
                Some(v) => v.to_string(),
                None => latest_neoforge(mc)?,
            };
            let url = format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{version}/neoforge-{version}-installer.jar");
            run_installer("NeoForge", &url, &version, &format!("neoforge-{version}"), "--install-client", java_component)
        }
        "forge" => {
            let version = match loader_version {
                Some(v) => v.to_string(),
                None => latest_forge(mc)?,
            };
            let url = format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{version}/forge-{mc}-{version}-installer.jar");
            run_installer("Forge", &url, &version, &format!("{mc}-forge-{version}"), "--installClient", java_component)
        }
        other => bail!("unknown modloader \"{other}\" (try one of: {LOADERS})"),
    }
}

/// Fabric and Quilt: ask their meta server for the newest stable loader, and save the profile it generates.
fn meta_profile(name: &str, api: &str, mc: &str, loader_version: Option<&str>) -> Result<String> {
    let version = match loader_version {
        Some(v) => v.to_string(),
        None => {
            let list = get_json(&format!("{api}/versions/loader/{mc}"))?;
            let list = list.as_array().filter(|l| !l.is_empty()).with_context(|| format!("{name} does not support Minecraft {mc}"))?;
            // Newest first. Fabric marks releases as "stable"; Quilt marks prereleases with a "-" in the version
            list.iter()
                .map(|e| &e["loader"])
                .find(|l| l["stable"].as_bool().unwrap_or_else(|| !l["version"].as_str().unwrap_or("-").contains('-')))
                .and_then(|l| l["version"].as_str())
                .with_context(|| format!("{name} has no stable loader for Minecraft {mc}"))?
                .to_string()
        }
    };
    let profile = get_json(&format!("{api}/versions/loader/{mc}/{version}/profile/json"))
        .with_context(|| format!("{name} loader {version} for Minecraft {mc}"))?;
    let id = profile["id"].as_str().context("the profile has no id")?.to_string();
    let dir = paths::versions()?.join(&id);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(format!("{id}.json")), serde_json::to_string_pretty(&profile)?)?;
    say!("{name} loader {version}");
    Ok(id)
}

/// "1.21.11" -> "21.11.", "1.21" -> "21.0.", "26.3" -> "26.3."
fn neoforge_prefix(mc: &str) -> String {
    let rest = mc.strip_prefix("1.").unwrap_or(mc);
    if rest.contains('.') { format!("{rest}.") } else { format!("{rest}.0.") }
}

fn numbers(version: &str) -> Vec<u64> {
    version.split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).collect()
}

fn latest_neoforge(mc: &str) -> Result<String> {
    let all = get_json("https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge")?;
    let prefix = neoforge_prefix(mc);
    let mut found: Vec<&str> = all["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|v| v.starts_with(&prefix))
        .collect();
    // Betas only if there is nothing else
    if found.iter().any(|v| !v.contains('-')) {
        found.retain(|v| !v.contains('-'));
    }
    found
        .into_iter()
        .max_by_key(|v| numbers(v))
        .map(str::to_string)
        .with_context(|| format!("NeoForge has no build for Minecraft {mc}"))
}

fn latest_forge(mc: &str) -> Result<String> {
    let promos = get_json("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")?;
    ["recommended", "latest"]
        .iter()
        .find_map(|kind| promos["promos"][format!("{mc}-{kind}")].as_str())
        .map(str::to_string)
        .with_context(|| format!("Forge has no build for Minecraft {mc}"))
}

/// Forge and NeoForge: download the installer and run it headless against mcterm's own folder,
/// which is laid out like a `.minecraft` (versions/, libraries/).
fn run_installer(name: &str, url: &str, version: &str, profile_id: &str, flag: &str, java_component: &str) -> Result<String> {
    let root = paths::root()?;
    let profile_json = paths::versions()?.join(profile_id).join(format!("{profile_id}.json"));
    if profile_json.exists() {
        say!("{name} {version} is already installed");
        return Ok(profile_id.to_string());
    }

    let installer: PathBuf = root.join("installers").join(format!("{name}-{version}-installer.jar").to_lowercase());
    download_one(url, &installer, None).with_context(|| format!("{name} {version} (does it support this Minecraft version?)"))?;
    // The installers expect a launcher to have been here before
    let launcher_profiles = root.join("launcher_profiles.json");
    if !launcher_profiles.exists() {
        fs::write(&launcher_profiles, r#"{"profiles":{}}"#)?;
    }

    let java = runtime::ensure_java(java_component)?;
    say!("running the {name} {version} installer (a minute or two)");
    let output = Command::new(&java)
        .arg("-jar")
        .arg(&installer)
        .arg(flag)
        .arg(&root)
        .current_dir(&root)
        .output()
        .with_context(|| format!("could not start {}", java.display()))?;
    if !output.status.success() || !profile_json.exists() {
        let log = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = log.lines().chain(err.lines()).collect();
        let tail = tail[tail.len().saturating_sub(15)..].join("\n");
        bail!("the {name} installer failed ({}):\n{tail}", output.status);
    }
    Ok(profile_id.to_string())
}
