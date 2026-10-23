//! Saved accounts, in `accounts.json`: who you are in the game, and which account is active.
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub name: String,
    /// 32 hex digits, no dashes
    pub uuid: String,
    /// "offline" or "microsoft"
    pub kind: String,
    /// Microsoft accounts only: what renews the login without asking again
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Microsoft accounts only: what the game is handed, and when (unix seconds) it stops working
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default)]
    pub expires_at: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Accounts {
    pub active: Option<String>,
    pub accounts: Vec<Account>,
}

/// Only you may read what's in here: `accounts.json` will hold login tokens one day.
const PRIVATE_FILE: u32 = 0o600;
const PRIVATE_DIR: u32 = 0o700;

fn restrict(path: &Path, mode: u32) -> Result<()> {
    if fs::metadata(path)?.permissions().mode() & 0o777 != mode {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

impl Accounts {
    pub fn load() -> Result<Self> {
        let path = paths::root()?.join("accounts.json");
        match fs::read(&path) {
            Ok(bytes) => {
                // A file written by an older mcterm, or touched by something else, may be open to others
                restrict(&path, PRIVATE_FILE)?;
                serde_json::from_slice(&bytes).context("accounts.json is damaged")
            }
            Err(_) => Ok(Self::default()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let root = paths::root()?;
        fs::create_dir_all(&root)?;
        restrict(&root, PRIVATE_DIR)?;
        // Write a private temp file, then rename it into place: the data is never readable by others, even for a moment
        let path = root.join("accounts.json");
        let temp = root.join("accounts.json.tmp");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(PRIVATE_FILE).open(&temp)?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        file.sync_all()?;
        restrict(&temp, PRIVATE_FILE)?;
        fs::rename(&temp, &path)?;
        Ok(())
    }

    /// Adds an account, or replaces the one with the same name, and makes it the active one.
    pub fn upsert_active(&mut self, account: Account) {
        self.active = Some(account.name.clone());
        match self.accounts.iter_mut().find(|a| a.name == account.name) {
            Some(existing) => *existing = account,
            None => self.accounts.push(account),
        }
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| Some(&a.name) == self.active.as_ref())
    }
}

/// The UUID Minecraft gives offline players: a version-3 UUID of "OfflinePlayer:<name>".
fn offline_uuid(name: &str) -> String {
    let mut bytes = md5::compute(format!("OfflinePlayer:{name}")).0;
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn dashed(uuid: &str) -> String {
    format!("{}-{}-{}-{}-{}", &uuid[..8], &uuid[8..12], &uuid[12..16], &uuid[16..20], &uuid[20..])
}

pub fn add_offline(name: &str) -> Result<()> {
    if !(3..=16).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("a player name is 3 to 16 letters, digits or _ (got \"{name}\")");
    }
    let mut accounts = Accounts::load()?;
    if !accounts.accounts.iter().any(|a| a.name == name) {
        accounts.accounts.push(Account { name: name.to_string(), uuid: offline_uuid(name), kind: "offline".into(), refresh_token: None, access_token: None, expires_at: 0 });
    }
    accounts.active = Some(name.to_string());
    accounts.save()?;
    say!("offline account \"{name}\" is ready and active");
    Ok(())
}

pub fn switch(name: &str) -> Result<()> {
    let mut accounts = Accounts::load()?;
    if !accounts.accounts.iter().any(|a| a.name == name) {
        bail!("no account \"{name}\" (see `mcterm auth list-accounts`)");
    }
    accounts.active = Some(name.to_string());
    accounts.save()?;
    say!("now playing as \"{name}\"");
    Ok(())
}

pub fn list() -> Result<()> {
    let accounts = Accounts::load()?;
    if accounts.accounts.is_empty() {
        say!("No accounts yet. Try `mcterm auth --offline NAME`.");
    }
    for a in &accounts.accounts {
        let mark = if Some(&a.name) == accounts.active.as_ref() { "*" } else { " " };
        say!("{mark} {:<16} {:<8} {}", a.name, a.kind, dashed(&a.uuid));
    }
    Ok(())
}
