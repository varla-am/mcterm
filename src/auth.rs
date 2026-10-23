//! `mcterm auth --online`: sign in with a Microsoft account, the way the official launcher does.
//!
//! 1. Microsoft's device code flow: we show a code, you type it in the browser, we get a Microsoft token.
//! 2. Xbox Live, then XSTS: the Microsoft token is swapped for Xbox tokens.
//! 3. Minecraft Services: the Xbox token is swapped for the Minecraft token the game wants, and your profile.
//! No password ever passes through mcterm.
use std::fs;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::accounts::{Account, Accounts};
use crate::paths;

const MICROSOFT: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const SCOPE: &str = "XboxLive.signin offline_access";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The Azure app to sign in with: `$MCTERM_CLIENT_ID`, or the one saved by `mcterm config client-id`.
fn client_id() -> Result<String> {
    if let Ok(id) = std::env::var("MCTERM_CLIENT_ID") {
        return Ok(id);
    }
    let config: Value = fs::read(paths::root()?.join("config.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    config["client_id"]
        .as_str()
        .map(str::to_string)
        .context("no client ID: register an Azure app, then run `mcterm config client-id <ID>` (or set MCTERM_CLIENT_ID)")
}

pub fn config_client_id(value: Option<&str>) -> Result<()> {
    let path = paths::root()?.join("config.json");
    let Some(id) = value else {
        match client_id() {
            Ok(id) => say!("client ID: {id}"),
            Err(_) => say!("no client ID set"),
        }
        return Ok(());
    };
    if id.len() != 36 || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        bail!("that doesn't look like a client ID (it should be like 00000000-0000-0000-0000-000000000000)");
    }
    fs::create_dir_all(paths::root()?)?;
    fs::write(&path, serde_json::to_string_pretty(&json!({ "client_id": id }))?)?;
    say!("saved the client ID");
    Ok(())
}

fn http() -> Result<Client> {
    Ok(Client::builder().user_agent(concat!("mcterm/", env!("CARGO_PKG_VERSION"))).timeout(Duration::from_secs(30)).build()?)
}

fn post_json(http: &Client, url: &str, body: &Value) -> Result<(u16, Value)> {
    let response = http.post(url).header("Accept", "application/json").json(body).send().with_context(|| format!("could not reach {url}"))?;
    let status = response.status().as_u16();
    Ok((status, response.json().unwrap_or(Value::Null)))
}

fn post_form(http: &Client, url: &str, form: &[(&str, &str)]) -> Result<(u16, Value)> {
    let response = http.post(url).form(form).send().with_context(|| format!("could not reach {url}"))?;
    let status = response.status().as_u16();
    Ok((status, response.json().unwrap_or(Value::Null)))
}

/// What Xbox says when it won't hand out a token, in words.
fn xsts_problem(code: u64) -> &'static str {
    match code {
        2148916233 => "this Microsoft account has no Xbox account; sign in once at xbox.com to create one",
        2148916235 => "Xbox Live isn't available in this account's country",
        2148916236 | 2148916237 => "this account needs adult verification",
        2148916238 => "this is a child account: an adult has to add it to a Microsoft family group, and allow online play",
        _ => "Xbox refused to sign this account in",
    }
}

struct Login {
    refresh_token: String,
    minecraft_token: String,
    expires_in: u64,
    uuid: String,
    name: String,
}

/// Microsoft token -> Xbox Live -> XSTS -> Minecraft token and profile.
fn minecraft_login(http: &Client, microsoft_token: &str, refresh_token: String) -> Result<Login> {
    let (status, xbl) = post_json(
        http,
        "https://user.auth.xboxlive.com/user/authenticate",
        &json!({
            "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={microsoft_token}") },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }),
    )?;
    let xbl_token = xbl["Token"].as_str().with_context(|| format!("Xbox Live sign-in failed (HTTP {status})"))?;

    let (status, xsts) = post_json(
        http,
        "https://xsts.auth.xboxlive.com/xsts/authorize",
        &json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }),
    )?;
    let Some(xsts_token) = xsts["Token"].as_str() else {
        match xsts["XErr"].as_u64() {
            Some(code) => bail!("{} (Xbox error {code})", xsts_problem(code)),
            None => bail!("Xbox authorization failed (HTTP {status})"),
        }
    };
    let user_hash = xsts["DisplayClaims"]["xui"][0]["uhs"].as_str().context("Xbox sent no user hash")?;

    let (status, minecraft) = post_json(
        http,
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        &json!({ "identityToken": format!("XBL3.0 x={user_hash};{xsts_token}") }),
    )?;
    let Some(minecraft_token) = minecraft["access_token"].as_str() else {
        if status == 403 {
            bail!("Minecraft refused this app (HTTP 403): its client ID has to be approved by Mojang first");
        }
        bail!("Minecraft sign-in failed (HTTP {status})");
    };

    let response = http.get("https://api.minecraftservices.com/minecraft/profile").bearer_auth(minecraft_token).send()?;
    if response.status().as_u16() == 404 {
        bail!("this Microsoft account doesn't own Minecraft: Java Edition");
    }
    let profile: Value = response.error_for_status()?.json()?;
    Ok(Login {
        refresh_token,
        minecraft_token: minecraft_token.to_string(),
        expires_in: minecraft["expires_in"].as_u64().unwrap_or(3600),
        uuid: profile["id"].as_str().context("no profile id")?.to_string(),
        name: profile["name"].as_str().context("no profile name")?.to_string(),
    })
}

fn to_account(login: Login) -> Account {
    Account {
        name: login.name,
        uuid: login.uuid,
        kind: "microsoft".into(),
        refresh_token: Some(login.refresh_token),
        access_token: Some(login.minecraft_token),
        expires_at: now() + login.expires_in,
    }
}

pub fn login_online() -> Result<()> {
    let client_id = client_id()?;
    let http = http()?;

    let (status, device) = post_form(&http, &format!("{MICROSOFT}/devicecode"), &[("client_id", &client_id), ("scope", SCOPE)])?;
    let (Some(device_code), Some(user_code), Some(url)) =
        (device["device_code"].as_str(), device["user_code"].as_str(), device["verification_uri"].as_str())
    else {
        bail!("Microsoft refused to start the sign-in (HTTP {status}): {}", device["error_description"].as_str().unwrap_or("no reason given"));
    };
    say!("open {url} and enter the code: {user_code}");
    let _ = Command::new("open").arg(url).status();

    // Wait for the sign-in to be finished in the browser
    let mut interval = device["interval"].as_u64().unwrap_or(5);
    let deadline = now() + device["expires_in"].as_u64().unwrap_or(900);
    let tokens = loop {
        sleep(Duration::from_secs(interval));
        if now() > deadline {
            bail!("the code expired; run `mcterm auth --online` again");
        }
        let (_, answer) = post_form(
            &http,
            &format!("{MICROSOFT}/token"),
            &[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", &client_id), ("device_code", device_code)],
        )?;
        match answer["error"].as_str() {
            None => break answer,
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some(other) => bail!("sign-in failed: {other} ({})", answer["error_description"].as_str().unwrap_or("")),
        }
    };

    say!("signed in with Microsoft, now asking Xbox and Minecraft");
    let access = tokens["access_token"].as_str().context("Microsoft sent no token")?;
    let refresh = tokens["refresh_token"].as_str().context("Microsoft sent no refresh token")?;
    let account = to_account(minecraft_login(&http, access, refresh.to_string())?);
    let name = account.name.clone();
    let mut accounts = Accounts::load()?;
    accounts.upsert_active(account);
    accounts.save()?;
    say!("signed in as \"{name}\" and set as the active account");
    Ok(())
}

/// The active account, ready to play: a Microsoft login that's about to expire is renewed first.
pub fn active_ready() -> Result<Account> {
    let mut accounts = Accounts::load()?;
    let Some(account) = accounts.active_account().cloned() else {
        bail!("no active account: run `mcterm auth --offline NAME` or `mcterm auth --online` first");
    };
    if account.kind != "microsoft" || account.expires_at > now() + 300 {
        return Ok(account);
    }

    say!("renewing the login for {}", account.name);
    let refresh = account.refresh_token.as_deref().context("this account has no refresh token; sign in again with `mcterm auth --online`")?;
    let http = http()?;
    let (_, answer) = post_form(
        &http,
        &format!("{MICROSOFT}/token"),
        &[("grant_type", "refresh_token"), ("client_id", &client_id()?), ("refresh_token", refresh), ("scope", SCOPE)],
    )?;
    let (Some(access), Some(new_refresh)) = (answer["access_token"].as_str(), answer["refresh_token"].as_str()) else {
        bail!("the saved login no longer works ({}); sign in again with `mcterm auth --online`", answer["error"].as_str().unwrap_or("unknown error"));
    };
    let renewed = to_account(minecraft_login(&http, access, new_refresh.to_string())?);
    accounts.upsert_active(renewed.clone());
    accounts.save()?;
    Ok(renewed)
}
