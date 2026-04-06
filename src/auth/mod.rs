pub(crate) mod config;
mod credentials;
mod server;

use anyhow::{anyhow, Result};
use rand::Rng;
use std::time::Duration;

pub fn login() -> Result<()> {
    let state: String = rand::rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    let (port, rx) = server::start_callback_server(&state)?;

    let server_url = config::server_url();
    let login_url =
        format!("{server_url}/login?redirect_uri=http://localhost:{port}/callback&state={state}");

    eprintln!("Opening browser for login...");

    if open::that(&login_url).is_err() {
        eprintln!("Could not open browser. Please visit this URL manually:");
        eprintln!("  {login_url}");
    }

    eprintln!("Waiting for authentication...");

    let token = rx
        .recv_timeout(Duration::from_secs(120))
        .map_err(|_| anyhow!("Login timed out. Please try again."))?;

    credentials::save(&credentials::Credentials { token })?;

    eprintln!("Logged in successfully.");
    Ok(())
}

pub fn logout() -> Result<()> {
    credentials::delete()?;
    eprintln!("Logged out.");
    Ok(())
}

pub fn whoami() -> Result<()> {
    let creds = match credentials::load()? {
        Some(c) => c,
        None => {
            eprintln!("Not logged in. Run `replay login` to authenticate.");
            return Ok(());
        }
    };

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(format!("{}/api/auth/get-session", config::server_url()))
        .header("Authorization", format!("Bearer {}", creds.token))
        .header("X-Client-Version", env!("CARGO_PKG_VERSION"))
        .send()?;

    if !resp.status().is_success() {
        eprintln!("Session expired or invalid. Run `replay login` to re-authenticate.");
        return Ok(());
    }

    let body: serde_json::Value = resp.json()?;

    if let Some(user) = body.get("user") {
        let name = user
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let email = user
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        eprintln!("Logged in as {name} ({email})");
    } else {
        eprintln!("Authenticated, but could not read user info.");
    }

    Ok(())
}

/// Load the stored auth token, if any. For use by other modules (e.g. upload).
#[allow(dead_code)]
pub fn get_token() -> Result<Option<String>> {
    Ok(credentials::load()?.map(|c| c.token))
}

/// User info returned by the server.
pub struct UserInfo {
    pub name: String,
    pub email: String,
}

/// Check if the user is logged in and fetch their info.
/// Returns Ok(None) if not logged in or session is invalid.
pub fn get_user_info() -> Result<Option<UserInfo>> {
    let creds = match credentials::load()? {
        Some(c) => c,
        None => return Ok(None),
    };

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(format!("{}/api/auth/get-session", config::server_url()))
        .header("Authorization", format!("Bearer {}", creds.token))
        .header("X-Client-Version", env!("CARGO_PKG_VERSION"))
        .send()?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: serde_json::Value = resp.json()?;
    if let Some(user) = body.get("user") {
        let name = user
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let email = user
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(Some(UserInfo { name, email }))
    } else {
        Ok(None)
    }
}
