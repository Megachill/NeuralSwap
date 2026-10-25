//! Cover art, banners and game details from Steam's public store endpoints.
//! No account, no key. The only thing the app uses the network for.

use crate::state::ArtMeta;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn asset(id: u32, file: &str) -> String {
    format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{id}/{file}")
}

static LAST_CALL: Mutex<Option<Instant>> = Mutex::new(None);

/// Steam's store API is rate limited; keep a comfortable distance.
fn throttle() {
    let mut last = LAST_CALL.lock().unwrap();
    if let Some(t) = *last {
        let elapsed = t.elapsed();
        if elapsed < Duration::from_millis(320) {
            std::thread::sleep(Duration::from_millis(320) - elapsed);
        }
    }
    *last = Some(Instant::now());
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

fn get_bytes(url: &str) -> Option<Vec<u8>> {
    let mut res = agent().get(url).header("Accept-Language", "en").call().ok()?;
    res.body_mut().with_config().limit(32 * 1024 * 1024).read_to_vec().ok()
}

fn get_json(url: &str) -> Option<serde_json::Value> {
    throttle();
    serde_json::from_slice(&get_bytes(url)?).ok()
}

/// Folder names carry edition suffixes and scene tags no store matches.
pub fn clean_name(name: &str) -> String {
    let a = Regex::new(r"\[[^\]]*\]|\([^)]*\)").unwrap().replace_all(name, " ");
    let b = Regex::new(r"(?i)\b(repack|fitgirl|dodi|elamigos|codex|rune|empress|plaza|skidrow|multi\d*)\b").unwrap().replace_all(&a, " ");
    let c = Regex::new(r"[_\-–—:™®]+").unwrap().replace_all(&b, " ");
    Regex::new(r"\s+").unwrap().replace_all(&c, " ").trim().to_string()
}

fn norm(s: &str) -> String {
    let lower = s.to_lowercase();
    let a = Regex::new(r"[^a-z0-9 ]").unwrap().replace_all(&lower, " ");
    Regex::new(r"\s+").unwrap().replace_all(&a, " ").trim().to_string()
}

/// "Control" happily returns "Star Control", so the closest title wins.
fn find_app_id(name: &str) -> Option<u32> {
    let query = clean_name(name);
    if query.is_empty() {
        return None;
    }
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&cc=us&l=en",
        query.replace(' ', "%20").replace('&', "%26")
    );
    let j = get_json(&url)?;
    let q = norm(&query);
    let q_words: Vec<&str> = q.split(' ').collect();
    let mut best = None;
    let mut best_score = -1f64;
    for row in j["items"].as_array()? {
        if row["type"].as_str().map(|t| t != "app").unwrap_or(false) {
            continue;
        }
        let n = norm(row["name"].as_str().unwrap_or(""));
        let words: Vec<&str> = n.split(' ').collect();
        let shared = words.iter().filter(|w| q_words.contains(w)).count() as f64;
        let mut score = 0f64;
        if n == q {
            score += 120.0;
        } else if n.starts_with(&q) || q.starts_with(&n) {
            score += 70.0;
        }
        score += shared / q_words.len().max(1) as f64 * 40.0;
        score -= (words.len() as f64 - q_words.len() as f64).max(0.0) * 6.0;
        if score > best_score {
            best_score = score;
            best = row["id"].as_u64();
        }
    }
    if best_score > 10.0 { best.map(|b| b as u32) } else { None }
}

pub fn look_up(name: &str, appid: Option<u32>) -> ArtMeta {
    let mut meta = ArtMeta { looked_up: true, ..Default::default() };
    let Some(id) = appid.or_else(|| find_app_id(name)) else { return meta };
    meta.appid = Some(id);
    if let Some(j) = get_json(&format!("https://store.steampowered.com/api/appdetails?appids={id}&cc=us&l=english")) {
        let d = &j[id.to_string()]["data"];
        meta.title = d["name"].as_str().map(String::from);
        meta.summary = d["short_description"].as_str().map(|s| html_unescape(s));
        meta.year = d["release_date"]["date"].as_str().and_then(|s| Regex::new(r"\d{4}").unwrap().find(s).and_then(|m| m.as_str().parse().ok()));
        meta.genres = d["genres"].as_array().map(|a| a.iter().filter_map(|g| g["description"].as_str().map(String::from)).take(3).collect()).unwrap_or_default();
    }
    meta
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

pub fn cache_dir() -> PathBuf {
    let d = crate::state::data_dir().join("posters");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Downloads an asset into the cache once. A missing asset can come back as
/// a tiny placeholder rather than a 404, so those are refused.
pub fn cached_asset(id: u32, file: &str) -> Option<PathBuf> {
    let dest = cache_dir().join(format!("{id}_{file}"));
    if dest.exists() {
        return Some(dest);
    }
    let bytes = get_bytes(&asset(id, file))?;
    if bytes.len() < 2000 {
        return None;
    }
    std::fs::write(&dest, bytes).ok()?;
    Some(dest)
}

pub fn cover(id: u32) -> Option<PathBuf> {
    cached_asset(id, "library_600x900.jpg")
}

pub fn hero(id: u32) -> Option<PathBuf> {
    cached_asset(id, "library_hero.jpg").or_else(|| cached_asset(id, "header.jpg"))
}

/// Decodes and shrinks an image file to fit within max_w×max_h.
pub fn decode(path: &Path, max_w: u32, max_h: u32) -> Option<egui::ColorImage> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max_w || img.height() > max_h { img.thumbnail(max_w, max_h) } else { img };
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}
