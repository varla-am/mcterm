//! One Minecraft version's own description: its jar, libraries and assets.
use std::collections::{HashMap, HashSet};
use std::fs;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::paths;

#[derive(Deserialize)]
pub struct VersionJson {
    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,
    pub downloads: Downloads,
    pub libraries: Vec<Library>,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    /// Name of the asset index ("12", "29", "legacy", ...)
    pub assets: String,
    /// "release", "snapshot", ... as the game wants to be told
    #[serde(rename = "type")]
    pub kind: String,
    /// New-style (1.13+) arguments, with rules
    pub arguments: Option<Arguments>,
    /// Old-style arguments: one string, no rules
    #[serde(rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,
    /// Missing in old versions, which all ran on Java 8
    #[serde(rename = "javaVersion")]
    pub java_version: Option<JavaVersion>,
    pub logging: Option<Logging>,
}

#[derive(Deserialize)]
pub struct JavaVersion {
    /// Which of Mojang's runtimes to use, like "java-runtime-delta"
    pub component: String,
    #[serde(rename = "majorVersion")]
    pub major: u32,
}

#[derive(Deserialize)]
pub struct File {
    /// Where it goes under `libraries/` (only libraries have one)
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct Downloads {
    pub client: File,
}

#[derive(Deserialize)]
pub struct Logging {
    pub client: Option<LoggingClient>,
}

#[derive(Deserialize)]
pub struct LoggingClient {
    pub file: LoggingFile,
}

#[derive(Deserialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional { rules: Vec<Rule>, value: StringOrList },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl Argument {
    /// The words this argument adds on this Mac (none if its rules say no).
    pub fn values(&self) -> Vec<&str> {
        match self {
            Argument::Plain(s) => vec![s.as_str()],
            Argument::Conditional { rules, value } if rules_allow(rules) => match value {
                StringOrList::One(s) => vec![s.as_str()],
                StringOrList::Many(list) => list.iter().map(String::as_str).collect(),
            },
            Argument::Conditional { .. } => Vec::new(),
        }
    }
}

#[derive(Deserialize)]
pub struct Library {
    pub name: String,
    pub downloads: Option<LibraryDownloads>,
    /// Maven repository to fetch from, for libraries with no `downloads` (Fabric, Quilt)
    pub url: Option<String>,
    pub rules: Option<Vec<Rule>>,
    /// Old-style natives: OS name -> classifier, like "osx" -> "natives-osx"
    pub natives: Option<HashMap<String, String>>,
    pub extract: Option<Extract>,
}

#[derive(Deserialize)]
pub struct LibraryDownloads {
    pub artifact: Option<File>,
    pub classifiers: Option<HashMap<String, File>>,
}

#[derive(Deserialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Deserialize)]
pub struct Rule {
    pub action: String,
    pub os: Option<OsRule>,
    /// Rules about launcher features (demo mode, ...); none of them apply to libraries we install
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Deserialize)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}

/// What this machine calls itself in Mojang's rules.
const HOST_OS: &str = "osx";

fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
}

impl Rule {
    fn matches(&self) -> bool {
        if self.features.is_some() {
            return false;
        }
        let Some(os) = &self.os else { return true };
        // `version` is a regex for Windows releases; it never holds for a Mac
        os.version.is_none()
            && os.name.as_deref().map_or(true, |n| n == HOST_OS)
            && os.arch.as_deref().map_or(true, |a| a == host_arch())
    }
}

/// Mojang's rules: the last matching rule decides, and nothing matching means no.
fn rules_allow(rules: &[Rule]) -> bool {
    rules.iter().filter(|r| r.matches()).last().map_or(false, |r| r.action == "allow")
}

impl Library {
    /// No rules means yes.
    pub fn applies(&self) -> bool {
        self.rules.as_deref().map_or(true, rules_allow)
    }

    /// The jar of an old-style native library for this OS, if it has one.
    pub fn native_file(&self) -> Option<&File> {
        let key = self.natives.as_ref()?.get(HOST_OS)?;
        self.downloads.as_ref()?.classifiers.as_ref()?.get(key)
    }
}

/// A library jar to put on disk and on the classpath.
pub struct Artifact {
    /// Under `libraries/`
    pub path: String,
    /// Empty for jars an installer makes itself
    pub url: String,
    pub sha1: Option<String>,
}

/// "group:artifact:version[:classifier]" -> "group/artifact/version/artifact-version[-classifier].jar"
fn maven_path(name: &str) -> Option<String> {
    let name = name.split('@').next()?;
    let mut parts = name.split(':');
    let (group, artifact, version) = (parts.next()?, parts.next()?, parts.next()?);
    let classifier = parts.next().map(|c| format!("-{c}")).unwrap_or_default();
    Some(format!("{}/{artifact}/{version}/{artifact}-{version}{classifier}.jar", group.replace('.', "/")))
}

impl Library {
    pub fn artifact(&self) -> Option<Artifact> {
        match &self.downloads {
            Some(d) => {
                let file = d.artifact.as_ref()?;
                Some(Artifact {
                    path: file.path.clone().or_else(|| maven_path(&self.name))?,
                    url: file.url.clone(),
                    sha1: Some(file.sha1.clone()).filter(|h| !h.is_empty()),
                })
            }
            None => {
                let path = maven_path(&self.name)?;
                let base = self.url.as_deref().unwrap_or("https://libraries.minecraft.net/");
                Some(Artifact { url: format!("{}/{path}", base.trim_end_matches('/')), path, sha1: None })
            }
        }
    }
}

/// "group:artifact[:classifier]": two versions of the same library shouldn't both be on the classpath
fn library_key(lib: &Value) -> String {
    let name = lib["name"].as_str().unwrap_or_default();
    let mut parts = name.split('@').next().unwrap_or_default().split(':');
    let (group, artifact) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    format!("{group}:{artifact}:{}", parts.nth(1).unwrap_or_default())
}

/// A modloader's profile only says what it changes; lay it over the vanilla version it `inheritsFrom`.
fn merge(parent: Value, child: Value) -> Value {
    let mut out = parent.clone();
    let Some(out_map) = out.as_object_mut() else { return child };
    for (key, value) in child.as_object().into_iter().flatten() {
        match key.as_str() {
            "inheritsFrom" => {}
            // The loader's libraries come first, and replace the vanilla ones of the same name
            "libraries" => {
                let mut seen = HashSet::new();
                let libs: Vec<Value> = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(parent["libraries"].as_array().into_iter().flatten())
                    .filter(|l| seen.insert(library_key(l)))
                    .cloned()
                    .collect();
                out_map.insert(key.clone(), Value::Array(libs));
            }
            // Vanilla's arguments, then the loader's
            "arguments" => {
                let mut args = parent["arguments"].clone();
                for list in ["game", "jvm"] {
                    let extra = value[list].as_array().cloned().unwrap_or_default();
                    if extra.is_empty() {
                        continue;
                    }
                    let mut all = args[list].as_array().cloned().unwrap_or_default();
                    all.extend(extra);
                    args[list] = Value::Array(all);
                }
                out_map.insert(key.clone(), args);
            }
            _ => {
                out_map.insert(key.clone(), value.clone());
            }
        }
    }
    out
}

fn load_value(id: &str) -> Result<Value> {
    let path = paths::versions()?.join(id).join(format!("{id}.json"));
    let value: Value = serde_json::from_slice(&fs::read(&path).with_context(|| format!("could not read {}", path.display()))?)?;
    match value["inheritsFrom"].as_str() {
        Some(parent) => Ok(merge(load_value(parent)?, value)),
        None => Ok(value),
    }
}

/// A version as the game sees it: a loader's profile already merged into vanilla.
pub fn load(id: &str) -> Result<VersionJson> {
    serde_json::from_value(load_value(id)?).with_context(|| format!("{id} has an unexpected format"))
}
