//! `mcterm launch instance`: put together Java's command line, the way Mojang's launcher does, and run it.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::auth;
use crate::install::Instance;
use crate::paths;
use crate::runtime;
use crate::version;

#[derive(Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
    /// Very old versions want the assets laid out by name instead of by hash
    #[serde(default, rename = "virtual")]
    is_virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
}

fn replace_placeholders(arg: &str, values: &HashMap<&str, String>) -> String {
    let mut out = arg.to_string();
    for (key, value) in values {
        out = out.replace(&format!("${{{key}}}"), value);
    }
    out
}

/// Copies each asset to the place a very old game looks for it by name.
fn lay_out_assets(index: &AssetIndex, objects: &Path, to: &Path) -> Result<()> {
    for (name, object) in &index.objects {
        let dest = to.join(name);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(objects.join(&object.hash[..2]).join(&object.hash), &dest)?;
    }
    Ok(())
}

pub fn launch_instance(name: &str) -> Result<()> {
    let instance_dir = paths::instances()?.join(name);
    let instance: Instance = serde_json::from_slice(
        &fs::read(instance_dir.join("instance.json")).with_context(|| format!("no instance \"{name}\" (see `mcterm list instances`)"))?,
    )?;

    let account = auth::active_ready()?;
    let online = account.kind == "microsoft";

    let version_dir = paths::versions()?.join(&instance.version);
    // What to launch: the modloader's profile (which builds on vanilla), or vanilla itself
    let launch_id = instance.profile.as_deref().unwrap_or(&instance.version);
    let v = version::load(launch_id)?;

    let java_component = v.java_version.as_ref().map_or("jre-legacy", |j| j.component.as_str());
    say!("preparing Java ({java_component})");
    let java = runtime::ensure_java(java_component)?;

    // Classpath: every library that applies here, then the game itself
    let libraries = paths::libraries()?;
    let mut classpath: Vec<PathBuf> = Vec::new();
    for lib in v.libraries.iter().filter(|l| l.applies()) {
        if let Some(artifact) = lib.artifact() {
            classpath.push(libraries.join(artifact.path));
        }
    }
    classpath.push(version_dir.join(format!("{}.jar", instance.version)));
    if let Some(missing) = classpath.iter().find(|p| !p.exists()) {
        bail!("{} is missing; try removing the instance and installing it again", missing.display());
    }
    let classpath = classpath.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(":");

    // Assets
    let assets_dir = paths::assets()?;
    let index: AssetIndex = serde_json::from_slice(&fs::read(assets_dir.join("indexes").join(format!("{}.json", v.assets)))?)?;
    let game_dir = instance_dir.join("game");
    let mut assets_root = assets_dir.clone();
    let mut game_assets = assets_dir.clone();
    if index.is_virtual {
        game_assets = assets_dir.join("virtual").join(&v.assets);
        lay_out_assets(&index, &assets_dir.join("objects"), &game_assets)?;
        assets_root = game_assets.clone();
    }
    if index.map_to_resources {
        lay_out_assets(&index, &assets_dir.join("objects"), &game_dir.join("resources"))?;
    }

    let natives_dir = version_dir.join("natives");
    fs::create_dir_all(&natives_dir)?;
    fs::create_dir_all(&game_dir)?;

    let mut values: HashMap<&str, String> = HashMap::new();
    values.insert("auth_player_name", account.name.clone());
    values.insert("auth_uuid", account.uuid.clone());
    let token = account.access_token.clone().unwrap_or_else(|| "0".into());
    values.insert("auth_session", token.clone());
    values.insert("auth_access_token", token);
    values.insert("auth_xuid", "0".into());
    values.insert("clientid", "0".into());
    values.insert("user_type", if online { "msa" } else { "legacy" }.into());
    values.insert("user_properties", "{}".into());
    values.insert("version_name", launch_id.to_string());
    values.insert("version_type", v.kind.clone());
    values.insert("game_directory", game_dir.to_string_lossy().into_owned());
    values.insert("assets_root", assets_root.to_string_lossy().into_owned());
    values.insert("game_assets", game_assets.to_string_lossy().into_owned());
    values.insert("assets_index_name", v.assets.clone());
    values.insert("natives_directory", natives_dir.to_string_lossy().into_owned());
    values.insert("library_directory", libraries.to_string_lossy().into_owned());
    values.insert("classpath", classpath);
    values.insert("classpath_separator", ":".into());
    values.insert("launcher_name", "mcterm".into());
    values.insert("launcher_version", env!("CARGO_PKG_VERSION").into());

    let (jvm_args, game_args): (Vec<String>, Vec<String>) = match (&v.arguments, &v.minecraft_arguments) {
        (Some(args), _) => (
            args.jvm.iter().flat_map(|a| a.values()).map(|a| replace_placeholders(a, &values)).collect(),
            args.game.iter().flat_map(|a| a.values()).map(|a| replace_placeholders(a, &values)).collect(),
        ),
        (None, Some(old)) => (
            ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"].iter().map(|a| replace_placeholders(a, &values)).collect(),
            old.split_whitespace().map(|a| replace_placeholders(a, &values)).collect(),
        ),
        (None, None) => bail!("{} describes no game arguments", instance.version),
    };
    say!("launching \"{name}\" ({}) as {}", instance.version, account.name);
    let status = Command::new(&java)
        .args(&jvm_args)
        .arg(&v.main_class)
        .args(&game_args)
        .current_dir(&game_dir)
        .status()
        .with_context(|| format!("could not start {}", java.display()))?;
    say!("the game exited ({status})");
    Ok(())
}
