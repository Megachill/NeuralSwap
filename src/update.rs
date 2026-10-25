//! Auto-update from GitHub Releases — no backend. The latest release of a
//! public repository carries `update.json`, the app exe and the payload zip;
//! every download is checked against the SHA-256 in `update.json`.

use crate::dist::{self, UpdateManifest};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Baked in at build time: `set NEURALSWAP_REPO=owner/repo` before
/// `cargo build`. Settings → Updates can override it.
pub const DEFAULT_REPO: Option<&str> = option_env!("NEURALSWAP_REPO");

#[derive(Clone, Debug)]
pub struct Available {
    pub manifest: UpdateManifest,
    pub app_url: String,
    pub payload_url: Option<String>,
    pub release_page: String,
    /// The payload changed too, so it will be downloaded.
    pub payload_changed: bool,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into()
}

fn get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    agent()
        .get(url)
        .header("User-Agent", concat!("NeuralSwap/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => "No published release found (is the repository public, with a release?)".to_string(),
            ureq::Error::StatusCode(403) => "GitHub rate limit reached — try again in an hour".to_string(),
            other => other.to_string(),
        })
}

fn parse_version(v: &str) -> Vec<u64> {
    v.trim_start_matches(['v', 'V']).split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    parse_version(remote) > parse_version(local)
}

/// Asks GitHub for the latest release. Ok(None) means up to date.
pub fn check(repo: &str, local_payload: Option<&Path>) -> Result<Option<Available>, String> {
    let repo = repo.trim().trim_matches('/');
    if repo.split('/').count() != 2 {
        return Err("Repository must look like owner/name".into());
    }
    // NEURALSWAP_GITHUB_API points at a local mirror for testing.
    let api = std::env::var("NEURALSWAP_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
    let mut res = get(&format!("{}/repos/{repo}/releases/latest", api.trim_end_matches('/')))?;
    let release: serde_json::Value = serde_json::from_reader(res.body_mut().as_reader()).map_err(|e| e.to_string())?;
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let url_of = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false))
            .and_then(|a| a["browser_download_url"].as_str().map(String::from))
    };
    let manifest_url = url_of(dist::MANIFEST_ASSET).ok_or("The latest release has no update.json asset")?;
    let mut res = get(&manifest_url)?;
    let manifest: UpdateManifest = serde_json::from_reader(res.body_mut().as_reader()).map_err(|e| format!("update.json: {e}"))?;
    if !is_newer(&manifest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let app_url = url_of(&manifest.app.name).ok_or("The release is missing the app executable")?;
    // Exe-only releases publish no payload: the local one is kept as is.
    let payload_url = manifest.payload.as_ref().and_then(|p| url_of(&p.name));
    let payload_changed = payload_url.is_some()
        && manifest.payload.as_ref().map(|m| local_payload.map(|p| dist::payload_id(p) != m.id).unwrap_or(true)).unwrap_or(false);
    Ok(Some(Available {
        payload_url,
        payload_changed,
        app_url,
        release_page: release["html_url"].as_str().unwrap_or("").to_string(),
        manifest,
    }))
}

fn download(url: &str, dest: &Path, expected_sha: &str, label: &str, progress: &mut dyn FnMut(f32, String)) -> Result<(), String> {
    let mut res = get(url)?;
    let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut reader = res.body_mut().with_config().limit(4 * 1024 * 1024 * 1024).reader();
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut f = File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(if total > 0 { done as f32 / total as f32 } else { 0.0 }, format!("Downloading {label}  {} / {} MB", done >> 20, total >> 20));
    }
    drop(f);
    let got = dist::sha256_file(&part).map_err(|e| e.to_string())?;
    if !got.eq_ignore_ascii_case(expected_sha) {
        let _ = fs::remove_file(&part);
        return Err(format!("{label} failed verification (SHA-256 mismatch) — nothing was changed"));
    }
    let _ = fs::remove_file(dest);
    fs::rename(&part, dest).map_err(|e| e.to_string())
}

/// Downloads, verifies and swaps in the new version. The payload (when it
/// changed) is replaced first; the running exe is renamed aside and the new
/// one put in its place. Returns the exe to restart.
pub fn apply(av: &Available, payload_root: Option<&Path>, progress: &mut dyn FnMut(f32, String)) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("no exe folder")?.to_path_buf();
    let work = crate::state::data_dir().join("updates");
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    if let (true, Some(url), Some(asset)) = (av.payload_changed, &av.payload_url, &av.manifest.payload) {
        {
            let zip = work.join(&asset.name);
            download(url, &zip, &asset.sha256, "DLSS 5 payload", &mut |f, t| progress(f * 0.7, t))?;
            let root = payload_root.map(Path::to_path_buf).unwrap_or_else(|| exe_dir.join("payload"));
            let staging = PathBuf::from(format!("{}.new", root.display()));
            let _ = fs::remove_dir_all(&staging);
            dist::extract(&zip, false, &staging, &mut |d, t| progress(0.7 + 0.15 * d as f32 / t.max(1) as f32, format!("Extracting payload  {} / {} MB", d >> 20, t >> 20)))
                .map_err(|e| e.to_string())?;
            let old = PathBuf::from(format!("{}.old", root.display()));
            let _ = fs::remove_dir_all(&old);
            if root.exists() {
                fs::rename(&root, &old).map_err(|e| format!("Could not replace the payload: {e}"))?;
            }
            fs::rename(&staging, &root).map_err(|e| e.to_string())?;
            let _ = fs::remove_dir_all(&old);
            let _ = fs::remove_file(&zip);
        }
    }

    let new_exe = work.join(&av.manifest.app.name);
    download(&av.app_url, &new_exe, &av.manifest.app.sha256, "NeuralSwap", &mut |f, t| progress(0.85 + f * 0.13, t))?;
    let aside = PathBuf::from(format!("{}.old", exe.display()));
    let _ = fs::remove_file(&aside);
    // Windows lets a running image be renamed, not overwritten.
    fs::rename(&exe, &aside).map_err(|e| format!("Could not move the running app aside: {e}"))?;
    if let Err(e) = fs::copy(&new_exe, &exe) {
        let _ = fs::rename(&aside, &exe);
        return Err(format!("Could not write the new version: {e}"));
    }
    let _ = fs::remove_file(&new_exe);
    progress(1.0, format!("Updated to {}", av.manifest.version));
    Ok(exe)
}
