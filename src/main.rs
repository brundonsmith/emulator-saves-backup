use anyhow::{Context, Result, anyhow};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use walkdir::WalkDir;

fn main() -> Result<()> {
    // Load .env file if it exists, otherwise fall back to normal env vars
    let _ = dotenvy::dotenv();

    let local_root = PathBuf::from(env::var("SYNC_LOCAL").context("SYNC_LOCAL must be set")?);
    let remote_root = env::var("SYNC_REMOTE").context("SYNC_REMOTE must be set")?;
    let client = Client::new();
    let token = get_access_token(&client)?;

    for entry in WalkDir::new(&local_root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let local_path = entry.path();
        let rel = local_path
            .strip_prefix(&local_root)
            .context("Failed to relativize path")?;
        let remote_path = format!(
            "{}/{}",
            remote_root.trim_end_matches('/'),
            rel.to_string_lossy().replace('\\', "/")
        );

        if let Err(e) = upload_file(&client, &token, local_path, &remote_path) {
            eprintln!("ERROR: {} → {}: {e}", local_path.display(), remote_path);
        } else {
            println!("Uploaded: {}", remote_path);
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
}

fn get_access_token(client: &Client) -> Result<String> {
    if let Ok(token) = env::var("DROPBOX_ACCESS_TOKEN") {
        return Ok(token);
    }

    let key = env::var("DROPBOX_APP_KEY")?;
    let secret = env::var("DROPBOX_APP_SECRET")?;
    let refresh = env::var("DROPBOX_REFRESH_TOKEN")?;

    let resp = send_with_retries(
        || {
            client
                .post("https://api.dropbox.com/oauth2/token")
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.as_str()),
                    ("client_id", key.as_str()),
                    ("client_secret", secret.as_str()),
                ])
                .send()
        },
        "getting access token",
    )?;

    let token_resp = resp.error_for_status()?.json::<TokenResp>()?;
    Ok(token_resp.access_token)
}

#[derive(Serialize)]
struct ApiArg<'a> {
    path: &'a str,
    mode: WriteMode,
    autorename: bool,
    mute: bool,
    strict_conflict: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase", tag = ".tag")]
enum WriteMode {
    Overwrite,
}

fn upload_file(client: &Client, token: &str, local: &Path, remote: &str) -> Result<()> {
    let mut file = File::open(local)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let arg = ApiArg {
        path: remote,
        mode: WriteMode::Overwrite,
        autorename: true,
        mute: true,
        strict_conflict: false,
    };
    let arg_json = serde_json::to_string(&arg)?;

    // buf is cloned per attempt; fine for your tiny files and simple code.
    let resp = send_with_retries(
        || {
            client
                .post("https://content.dropboxapi.com/2/files/upload")
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/octet-stream")
                .header("Dropbox-API-Arg", arg_json.clone())
                .body(buf.clone())
                .send()
        },
        &format!("uploading {}", remote),
    )?;

    resp.error_for_status()?;
    Ok(())
}

/// Retry helper: retries on connect/DNS/timeout errors with backoff.
fn send_with_retries<F>(mut make_req: F, what: &str) -> Result<Response>
where
    F: FnMut() -> Result<Response, reqwest::Error>,
{
    let delay = Duration::from_secs(5);
    let max_attempts = 5;

    for attempt in 1..=max_attempts {
        match make_req() {
            Ok(resp) => return Ok(resp),
            Err(e) if e.is_connect() || e.is_timeout() => {
                eprintln!(
                    "[deckpush] {} attempt {}/{} failed: {}. Retrying in {:?}…",
                    what, attempt, max_attempts, e, delay
                );
                thread::sleep(delay);
            }
            Err(e) => {
                // Non-retriable error (4xx, bad URL, etc.)
                return Err(e.into());
            }
        }
    }

    Err(anyhow!(
        "[deckpush] {}: giving up after {} attempts",
        what,
        max_attempts
    ))
}
