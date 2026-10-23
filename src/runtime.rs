//! Java: the runtime Mojang ships for each Minecraft version, downloaded once into `runtimes/`.
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::download::{download_all, Job};
use crate::paths;

const INDEX_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Mojang's names for this machine, best first. Apple Silicon falls back to the Intel build (it runs under Rosetta).
fn platforms(os: &str, arch: &str) -> &'static [&'static str] {
    match (os, arch) {
        ("macos", "aarch64") => &["mac-os-arm64", "mac-os"],
        ("macos", _) => &["mac-os"],
        ("linux", "x86_64") => &["linux"],
        ("linux", "x86") => &["linux-i386"],
        // Mojang ships no Java for the rest (Linux on ARM, ...)
        _ => &[],
    }
}

/// Where `java` sits inside a runtime: a macOS bundle, or plain folders.
fn java_in(dir: &Path, os: &str) -> PathBuf {
    if os == "macos" { dir.join("jre.bundle/Contents/Home/bin/java") } else { dir.join("bin/java") }
}

/// Java's major version from the output of `java -version`: "1.8.0_402" is 8, "21.0.2" is 21.
fn major_from(output: &str) -> Option<u32> {
    let version = output.split('"').nth(1)?;
    let mut parts = version.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

/// For machines Mojang has no Java for: the one the user installed (`$JAVA_HOME`, or `java` on the PATH).
fn system_java(major: u32) -> Result<PathBuf> {
    let java = std::env::var_os("JAVA_HOME").map(|home| PathBuf::from(home).join("bin/java")).filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("java"));
    let output = Command::new(&java).arg("-version").output().with_context(|| {
        format!("Mojang has no Java runtime for this machine, and no Java was found: install Java {major} (or set JAVA_HOME)")
    })?;
    // `java -version` prints to stderr
    let text = String::from_utf8_lossy(&output.stderr);
    match major_from(&text) {
        Some(found) if found == major => Ok(java),
        Some(found) => bail!("this version of Minecraft needs Java {major}, but the Java on this machine is {found}: install Java {major} (or set JAVA_HOME)"),
        None => bail!("could not tell which Java {} is", java.display()),
    }
}

/// Returns the `java` executable for a runtime component like "java-runtime-delta", installing it if needed.
/// `major` is the Java version the game needs; it only matters where Mojang has no runtime to give.
pub fn ensure_java(component: &str, major: u32) -> Result<PathBuf> {
    let dir = paths::root()?.join("runtimes").join(component);
    let java = java_in(&dir, std::env::consts::OS);
    if java.exists() {
        return Ok(java);
    }

    let index: Value = reqwest::blocking::get(INDEX_URL)?.error_for_status()?.json()?;
    let manifest_url = platforms(std::env::consts::OS, std::env::consts::ARCH)
        .iter()
        .find_map(|p| index[*p][component][0]["manifest"]["url"].as_str());
    let Some(manifest_url) = manifest_url else {
        return system_java(major);
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_java_version() {
        assert_eq!(major_from("openjdk version \"21.0.2\" 2024-01-16"), Some(21));
        assert_eq!(major_from("java version \"1.8.0_402\""), Some(8));
        assert_eq!(major_from("openjdk version \"17\" 2021-09-14"), Some(17));
        assert_eq!(major_from("no version here"), None);
    }

    #[test]
    fn picks_mojangs_platform() {
        assert_eq!(platforms("linux", "x86_64"), ["linux"]);
        assert_eq!(platforms("linux", "x86"), ["linux-i386"]);
        assert_eq!(platforms("macos", "aarch64"), ["mac-os-arm64", "mac-os"]);
        assert!(platforms("linux", "aarch64").is_empty());
    }

    #[test]
    fn java_sits_where_the_system_puts_it() {
        let dir = Path::new("/r/java-runtime-delta");
        assert_eq!(java_in(dir, "linux"), Path::new("/r/java-runtime-delta/bin/java"));
        assert_eq!(java_in(dir, "macos"), Path::new("/r/java-runtime-delta/jre.bundle/Contents/Home/bin/java"));
    }
}
