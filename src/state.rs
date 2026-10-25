//! Persistent settings, per-game choices and the scan cache, kept under
//! %APPDATA%\NeuralSwap.

use crate::core::scan::GameScan;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Bump when detection changes so an old wrong scan result is not kept.
pub const SCAN_RULES: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GamePrefs {
    /// The executable the user picked (relative to the game folder).
    pub exe_rel: Option<String>,
    /// "auto" or a rendering-API choice value (d3d11, d3d12…).
    pub api: Option<String>,
    pub route: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ArtMeta {
    pub appid: Option<u32>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub year: Option<u32>,
    pub genres: Vec<String>,
    /// True once a lookup ran, even when nothing was found.
    pub looked_up: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub payload_dir: Option<PathBuf>,
    pub folders: Vec<PathBuf>,
    pub manual_games: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
    pub companions: Vec<PathBuf>,
    pub per_game: HashMap<String, GamePrefs>,
    pub art: HashMap<String, ArtMeta>,
    pub fetch_art: bool,
    pub hide_unsupported: bool,
    /// GitHub "owner/repo" to update from; falls back to the built-in one.
    pub update_repo: Option<String>,
    pub auto_update_check: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            payload_dir: None,
            folders: Vec::new(),
            manual_games: Vec::new(),
            excluded: Vec::new(),
            companions: Vec::new(),
            per_game: HashMap::new(),
            art: HashMap::new(),
            fetch_art: true,
            hide_unsupported: false,
            update_repo: None,
            auto_update_check: true,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct ScanCache {
    pub rules: u32,
    pub scans: HashMap<String, GameScan>,
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."));
    let dir = base.join("NeuralSwap");
    // Carry settings, history and artwork over from before the rename.
    let old = base.join("DLSS5Swapper");
    if !dir.exists() && old.is_dir() && fs::rename(&old, &dir).is_err() {
        // A handle still open in the old folder blocks the rename: copy.
        copy_dir(&old, &dir);
    }
    let _ = fs::create_dir_all(&dir);
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    let _ = fs::create_dir_all(to);
    let Ok(entries) = fs::read_dir(from) else { return };
    for e in entries.flatten() {
        let target = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            let _ = fs::copy(e.path(), target);
        }
    }
}

pub fn game_key(dir: &Path) -> String {
    dir.to_string_lossy().to_lowercase().trim_end_matches('\\').to_string()
}

fn load<T: for<'de> Deserialize<'de> + Default>(file: &Path) -> T {
    // Tolerate a UTF-8 BOM: editors and PowerShell like to add one.
    fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(file: &Path, data: &T) {
    if let Ok(text) = serde_json::to_string_pretty(data) {
        let tmp = file.with_extension("tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, file);
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        load(&data_dir().join("settings.json"))
    }
    pub fn save(&self) {
        save(&data_dir().join("settings.json"), self)
    }
}

impl ScanCache {
    pub fn load() -> Self {
        let c: ScanCache = load(&data_dir().join("scans.json"));
        if c.rules == SCAN_RULES { c } else { ScanCache { rules: SCAN_RULES, scans: HashMap::new() } }
    }
    pub fn save(&self) {
        save(&data_dir().join("scans.json"), self)
    }
}
