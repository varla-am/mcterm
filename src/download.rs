//! Downloading many files at once, each checked against its SHA-1 when Mojang gives one.
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

use anyhow::{bail, Context, Result};
use sha1::{Digest, Sha1};

const WORKERS: usize = 16;
const TRIES: usize = 3;

pub struct Job {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
}

fn sha1_of(bytes: &[u8]) -> String {
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// True when the file is already there and (if we know the hash) intact.
fn is_fresh(job: &Job) -> bool {
    match (fs::read(&job.dest), &job.sha1) {
        (Ok(bytes), Some(want)) => sha1_of(&bytes).eq_ignore_ascii_case(want),
        (Ok(_), None) => true,
        _ => false,
    }
}

fn fetch(client: &reqwest::blocking::Client, job: &Job) -> Result<()> {
    let bytes = client.get(&job.url).send()?.error_for_status()?.bytes()?;
    if let Some(want) = &job.sha1 {
        let got = sha1_of(&bytes);
        if !got.eq_ignore_ascii_case(want) {
            bail!("hash mismatch (wanted {want}, got {got})");
        }
    }
    if let Some(parent) = job.dest.parent() {
        fs::create_dir_all(parent)?;
    }
    // Write beside the target and rename, so a half-finished download is never mistaken for a good file
    let mut part = job.dest.clone().into_os_string();
    part.push(".part");
    let part = PathBuf::from(part);
    fs::File::create(&part)?.write_all(&bytes)?;
    fs::rename(&part, &job.dest)?;
    Ok(())
}

/// Downloads every job that isn't already on disk, in parallel. `label` is shown in the progress line.
pub fn download_all(label: &str, jobs: Vec<Job>) -> Result<()> {
    let total = jobs.len();
    if total == 0 {
        return Ok(());
    }
    let client = reqwest::blocking::Client::builder().build()?;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failures: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let show_progress = std::io::stderr().is_terminal();

    thread::scope(|scope| {
        for _ in 0..WORKERS.min(total) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(i) else { break };
                if !is_fresh(job) {
                    let mut last = None;
                    for _ in 0..TRIES {
                        match fetch(&client, job) {
                            Ok(()) => {
                                last = None;
                                break;
                            }
                            Err(e) => last = Some(e),
                        }
                    }
                    if let Some(e) = last {
                        failures.lock().unwrap().push(format!("{}: {e:#}", job.url));
                    }
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if show_progress {
                    eprint!("\rmcterm: {label}: {n}/{total}");
                }
            });
        }
    });
    if show_progress {
        eprintln!();
    } else {
        eprintln!("mcterm: {label}: {total} files");
    }

    let failures = failures.into_inner().unwrap();
    if !failures.is_empty() {
        bail!("{} download(s) failed:\n  {}", failures.len(), failures.join("\n  "));
    }
    Ok(())
}

/// Downloads one file (skipping it if it's already fine).
pub fn download_one(url: &str, dest: &Path, sha1: Option<&str>) -> Result<()> {
    download_all(
        &dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        vec![Job { url: url.to_string(), dest: dest.to_path_buf(), sha1: sha1.map(str::to_string) }],
    )
    .with_context(|| format!("could not get {url}"))
}
