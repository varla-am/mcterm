//! `mcterm delete`: remove an instance, or one mod or resource pack from an instance.
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::mods;
use crate::paths;

fn dir_size(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path).into_iter().flatten().flatten().map(|e| dir_size(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b => format!("{} KB", b.div_ceil(1024)),
    }
}

fn confirm(question: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        bail!("this asks for confirmation; run it in a terminal, or add --yes");
    }
    print!("mcterm: {question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

pub fn delete_instance(name: &str, yes: bool) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        bail!("\"{name}\" is not an instance name");
    }
    let dir = paths::instances()?.join(name);
    if !dir.join("instance.json").exists() {
        bail!("no instance \"{name}\" (see `mcterm list instances`)");
    }
    let size = human(dir_size(&dir));
    if !yes && !confirm(&format!("Delete \"{name}\" with its mods, worlds and settings ({size})? This can't be undone."))? {
        say!("nothing deleted");
        return Ok(());
    }
    fs::remove_dir_all(&dir)?;
    say!("deleted instance \"{name}\" ({size})");
    Ok(())
}

/// Which instance a command is about: the one named, or the only one there is.
pub fn pick_instance(name: Option<&str>) -> Result<PathBuf> {
    let root = paths::instances()?;
    if let Some(name) = name {
        if name.contains(['/', '\\']) || name.starts_with('.') || !root.join(name).join("instance.json").exists() {
            bail!("no instance \"{name}\" (see `mcterm list instances`)");
        }
        return Ok(root.join(name));
    }
    let found: Vec<PathBuf> = fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("instance.json").exists())
        .collect();
    match found.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!("no instances yet"),
        _ => bail!("which instance? add --instance NAME (see `mcterm list instances`)"),
    }
}

/// One thing in a folder, with every name it can be called by (all lowercase).
struct Entry {
    shown: String,
    path: PathBuf,
    keys: Vec<String>,
}

fn entries_in(dir: &Path, what: &str, extra_keys: impl Fn(&Path) -> Vec<String>) -> Result<Vec<Entry>> {
    let mut all: Vec<Entry> = fs::read_dir(dir)
        .with_context(|| format!("there is no {what} folder yet ({})", dir.display()))?
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            let bare = file.rsplit_once('.').map_or(file.clone(), |(stem, _)| stem.to_string());
            let mut keys = vec![file.to_lowercase(), bare.to_lowercase()];
            let extra = extra_keys(&e.path());
            let shown = match extra.get(1) {
                Some(name) => format!("{file}  ({name})"),
                None => file,
            };
            keys.extend(extra.into_iter().map(|k| k.to_lowercase()));
            Entry { shown, path: e.path(), keys }
        })
        .collect();
    all.sort_by(|a, b| a.shown.cmp(&b.shown));
    Ok(all)
}

/// "sodium-0.6.jar", "Sodium-0.6", a mod's id or its display name all find the same file;
/// a piece of any of them works if it is unambiguous.
fn find_entry(entries: Vec<Entry>, wanted: &str, what: &str, dir: &Path) -> Result<PathBuf> {
    let wanted = wanted.to_lowercase();
    if let Some(e) = entries.iter().find(|e| e.keys.contains(&wanted)) {
        return Ok(e.path.clone());
    }
    let matches: Vec<&Entry> = entries.iter().filter(|e| e.keys.iter().any(|k| k.contains(&wanted))).collect();
    match matches.as_slice() {
        [e] => Ok(e.path.clone()),
        [] => bail!("no {what} matching \"{wanted}\" in {}", dir.display()),
        many => bail!("\"{wanted}\" matches more than one {what}:\n  {}", many.iter().map(|e| e.shown.as_str()).collect::<Vec<_>>().join("\n  ")),
    }
}

fn delete_path(path: &Path, what: &str) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    say!("deleted {what} {}", path.file_name().unwrap_or_default().to_string_lossy());
    Ok(())
}

pub fn delete_mod(instance: Option<&str>, name: &str) -> Result<()> {
    let dir = pick_instance(instance)?.join("game").join("mods");
    // A jar also answers to the id and name written inside it
    let entries = entries_in(&dir, "mod", |path| match mods::read_info(path) {
        Some(info) => vec![info.id, info.name],
        None => Vec::new(),
    })?;
    delete_path(&find_entry(entries, name, "mod", &dir)?, "mod")
}

pub fn delete_resourcepack(instance: Option<&str>, name: &str) -> Result<()> {
    let dir = pick_instance(instance)?.join("game").join("resourcepacks");
    let entries = entries_in(&dir, "resource pack", |_| Vec::new())?;
    delete_path(&find_entry(entries, name, "resource pack", &dir)?, "resource pack")
}
