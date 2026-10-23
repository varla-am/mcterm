//! `mcterm install mod`: find a mod on Modrinth, pick the right file for an instance, and put it in `mods/`.
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::delete::pick_instance;
use crate::download::download_one;
use crate::install::load_instance;
use crate::mods;

const API: &str = "https://api.modrinth.com/v2";

fn client() -> Result<Client> {
    // Modrinth asks every program to say who it is
    Ok(Client::builder().user_agent(concat!("mcterm/", env!("CARGO_PKG_VERSION"))).build()?)
}

fn get(client: &Client, path: &str, query: &[(&str, String)]) -> Result<Option<Value>> {
    let response = client.get(format!("{API}{path}")).query(query).send().with_context(|| format!("could not reach Modrinth ({path})"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.json()?))
}

/// What the instance can run: its Minecraft version, and the loaders whose mods fit (Quilt runs Fabric mods too).
struct Target {
    mc: String,
    loaders: Vec<&'static str>,
}

fn target_of(instance_dir: &Path) -> Result<Target> {
    let instance = load_instance(instance_dir)?;
    let loaders = match instance.modloader.as_deref() {
        Some("fabric") => vec!["fabric"],
        Some("quilt") => vec!["quilt", "fabric"],
        Some("neoforge") => vec!["neoforge"],
        Some("forge") => vec!["forge"],
        _ => bail!("\"{}\" is vanilla; install it with --modloader to use mods", instance.name),
    };
    Ok(Target { mc: instance.version, loaders })
}

/// A project by its slug, or else the best search result.
fn find_project(client: &Client, query: &str, target: &Target) -> Result<Value> {
    if !query.contains(' ') {
        if let Some(project) = get(client, &format!("/project/{}", query.to_lowercase()), &[])? {
            return Ok(project);
        }
    }
    let loader_facets: Vec<String> = target.loaders.iter().map(|l| format!("categories:{l}")).collect();
    let facets = json!([["project_type:mod"], [format!("versions:{}", target.mc)], loader_facets]);
    let found = get(client, "/search", &[("query", query.to_string()), ("facets", facets.to_string()), ("limit", "5".into())])?
        .context("Modrinth search failed")?;
    let hits = found["hits"].as_array().cloned().unwrap_or_default();
    let Some(best) = hits.first() else {
        bail!("no mod matching \"{query}\" for Minecraft {} with {}", target.mc, target.loaders.join("/"));
    };
    if hits.len() > 1 {
        say!("\"{query}\" also matches: {}", hits[1..].iter().filter_map(|h| h["slug"].as_str()).collect::<Vec<_>>().join(", "));
    }
    get(client, &format!("/project/{}", best["project_id"].as_str().unwrap_or_default()), &[])?.context("the project vanished")
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')
}

fn install_project(client: &Client, project: &Value, target: &Target, mods_dir: &Path, done: &mut HashSet<String>) -> Result<()> {
    let id = project["id"].as_str().context("a project has no id")?.to_string();
    if !done.insert(id.clone()) {
        return Ok(());
    }
    let title = project["title"].as_str().unwrap_or(&id);

    let versions = get(
        client,
        &format!("/project/{id}/version"),
        &[("game_versions", json!([target.mc]).to_string()), ("loaders", json!(target.loaders).to_string())],
    )?
    .and_then(|v| v.as_array().cloned())
    .unwrap_or_default();
    // Newest first; a release if there is one
    let version = versions
        .iter()
        .find(|v| v["version_type"] == "release")
        .or(versions.first())
        .with_context(|| format!("{title} has no version for Minecraft {} with {}", target.mc, target.loaders.join("/")))?;
    let files = version["files"].as_array().context("a version has no files")?;
    let file = files.iter().find(|f| f["primary"] == true).or(files.first()).context("a version has no files")?;
    let name = file["filename"].as_str().filter(|n| safe_file_name(n)).context("Modrinth gave an unsafe file name")?;
    let url = file["url"].as_str().context("a file has no url")?;
    let sha1 = file["hashes"]["sha1"].as_str();

    fs::create_dir_all(mods_dir)?;
    let dest = mods_dir.join(name);
    download_one(url, &dest, sha1)?;

    // Same mod, older file: that's an update, not a second copy
    if let Some(new) = mods::read_info(&dest) {
        for entry in fs::read_dir(mods_dir)?.flatten() {
            let path = entry.path();
            if path != dest && mods::read_info(&path).is_some_and(|old| old.id == new.id) {
                fs::remove_file(&path)?;
                say!("replaced {}", path.file_name().unwrap_or_default().to_string_lossy());
            }
        }
    }
    say!("installed {title} {} ({name})", version["version_number"].as_str().unwrap_or_default());

    // Other mods this one can't run without
    for dep in version["dependencies"].as_array().into_iter().flatten().filter(|d| d["dependency_type"] == "required") {
        let project_id = match (dep["project_id"].as_str(), dep["version_id"].as_str()) {
            (Some(p), _) => p.to_string(),
            (None, Some(v)) => match get(client, &format!("/version/{v}"), &[])? {
                Some(version) => version["project_id"].as_str().unwrap_or_default().to_string(),
                None => continue,
            },
            _ => continue,
        };
        if done.contains(&project_id) {
            continue;
        }
        if let Some(project) = get(client, &format!("/project/{project_id}"), &[])? {
            say!("{title} needs {}", project["title"].as_str().unwrap_or(&project_id));
            install_project(client, &project, target, mods_dir, done)?;
        }
    }
    Ok(())
}

pub fn install_mod(query: &str, instance: Option<&str>) -> Result<()> {
    let instance_dir = pick_instance(instance)?;
    let target = target_of(&instance_dir)?;
    let client = client()?;
    let project = find_project(&client, query, &target)?;
    install_project(&client, &project, &target, &instance_dir.join("game").join("mods"), &mut HashSet::new())
}

pub fn search_mods(query: &str, instance: Option<&str>) -> Result<()> {
    let target = target_of(&pick_instance(instance)?)?;
    let client = client()?;
    let loader_facets: Vec<String> = target.loaders.iter().map(|l| format!("categories:{l}")).collect();
    let facets = json!([["project_type:mod"], [format!("versions:{}", target.mc)], loader_facets]);
    let found = get(&client, "/search", &[("query", query.to_string()), ("facets", facets.to_string()), ("limit", "10".into())])?
        .context("Modrinth search failed")?;
    let hits = found["hits"].as_array().cloned().unwrap_or_default();
    if hits.is_empty() {
        say!("nothing found for \"{query}\"");
    }
    for h in hits {
        say!(
            "{:<24} {:<28} {:>10} downloads  {}",
            h["slug"].as_str().unwrap_or("?"),
            h["title"].as_str().unwrap_or("?"),
            h["downloads"].as_u64().unwrap_or(0),
            h["description"].as_str().unwrap_or("").chars().take(60).collect::<String>()
        );
    }
    Ok(())
}
