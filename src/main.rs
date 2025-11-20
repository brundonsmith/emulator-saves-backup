use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
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

    let resp = client
        .post("https://api.dropbox.com/oauth2/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", key.as_str()),
            ("client_secret", secret.as_str()),
        ])
        .send()?
        .error_for_status()?
        .json::<TokenResp>()?;

    Ok(resp.access_token)
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

    let request = client
        .post("https://content.dropboxapi.com/2/files/upload")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/octet-stream")
        .header("Dropbox-API-Arg", arg_json)
        .body(buf);

    // println!("{:?}", request.build().unwrap());

    request.send()?.error_for_status()?;

    Ok(())
}
