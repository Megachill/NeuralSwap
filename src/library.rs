//! Builds a games library out of what the launchers already keep on disk.
//! Everything here is a read: no network, no accounts, nothing written back.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    RRF_RT_REG_SZ,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LibraryGame {
    pub launcher: String,
    pub id: Option<String>,
    pub name: String,
    pub dir: PathBuf,
    /// Steam's own cached cover, when the client already downloaded one.
    pub poster: Option<PathBuf>,
    pub steam_appid: Option<u32>,
}

// Entries that are tooling rather than games.
fn not_a_game(name: &str) -> bool {
    Regex::new(r"(?i)redistributabl|steamworks common|directx|vcredist|proton|steam linux runtime|soundtrack|dedicated server|sdk$")
        .unwrap()
        .is_match(name)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let k = wide(key);
    let v = wide(value);
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size)
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

fn reg_subkeys(root: HKEY, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let k = wide(key);
    let mut h: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(root, k.as_ptr(), 0, KEY_READ, &mut h) } != ERROR_SUCCESS {
        return out;
    }
    let mut i = 0;
    loop {
        let mut name = [0u16; 512];
        let mut len = name.len() as u32;
        let rc = unsafe {
            RegEnumKeyExW(h, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
        };
        if rc != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
        i += 1;
    }
    unsafe { RegCloseKey(h) };
    out
}

fn kv<'a>(text: &'a str, key: &str) -> Option<String> {
    Regex::new(&format!(r#"(?i)"{key}"\s+"([^"]+)""#)).unwrap().captures(text).map(|c| c[1].to_string())
}

/// Steam keeps posters it already downloaded; newer clients nest them in
/// hashed subfolders.
fn steam_poster(root: &Path, appid: &str) -> Option<PathBuf> {
    let dir = root.join("appcache").join("librarycache").join(appid);
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if e.file_name().to_string_lossy().eq_ignore_ascii_case("library_600x900.jpg") {
                return Some(p);
            }
        }
    }
    let flat = root.join("appcache").join("librarycache").join(format!("{appid}_library_600x900.jpg"));
    flat.exists().then_some(flat)
}

pub fn steam_root() -> Option<PathBuf> {
    reg_string(HKEY_CURRENT_USER, "Software\\Valve\\Steam", "SteamPath")
        .map(|s| PathBuf::from(s.replace('/', "\\")))
        .filter(|p| p.join("steamapps").exists())
}

pub fn steam() -> Vec<LibraryGame> {
    let Some(base) = steam_root() else { return Vec::new() };
    let mut libraries = vec![base.clone()];
    if let Ok(vdf) = fs::read_to_string(base.join("steamapps").join("libraryfolders.vdf")) {
        for c in Regex::new(r#""path"\s+"([^"]+)""#).unwrap().captures_iter(&vdf) {
            let p = PathBuf::from(c[1].replace("\\\\", "\\"));
            if !libraries.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())) {
                libraries.push(p);
            }
        }
    }
    let mut games = Vec::new();
    for lib in libraries {
        let apps = lib.join("steamapps");
        let Ok(entries) = fs::read_dir(&apps) else { continue };
        for e in entries.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            if !(f.starts_with("appmanifest_") && f.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = fs::read_to_string(e.path()) else { continue };
            let (Some(appid), Some(installdir)) = (kv(&text, "appid"), kv(&text, "installdir")) else { continue };
            let name = kv(&text, "name").unwrap_or_else(|| installdir.clone());
            if not_a_game(&name) {
                continue;
            }
            let dir = apps.join("common").join(&installdir);
            if !dir.exists() {
                continue;
            }
            games.push(LibraryGame {
                launcher: "Steam".into(),
                poster: steam_poster(&base, &appid),
                steam_appid: appid.parse().ok(),
                id: Some(appid),
                name,
                dir,
            });
        }
    }
    games
}

pub fn epic() -> Vec<LibraryGame> {
    let dir = Path::new("C:\\ProgramData\\Epic\\EpicGamesLauncher\\Data\\Manifests");
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut games = Vec::new();
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().ends_with(".item") {
            continue;
        }
        let Ok(text) = fs::read_to_string(e.path()) else { continue };
        let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let (Some(loc), Some(name)) = (j["InstallLocation"].as_str(), j["DisplayName"].as_str()) else { continue };
        let dir = PathBuf::from(loc);
        if !dir.exists() || not_a_game(name) {
            continue;
        }
        games.push(LibraryGame {
            launcher: "Epic Games".into(),
            id: j["AppName"].as_str().map(String::from),
            name: name.into(),
            dir,
            poster: None,
            steam_appid: None,
        });
    }
    games
}

pub fn gog() -> Vec<LibraryGame> {
    let base = "SOFTWARE\\WOW6432Node\\GOG.com\\Games";
    let mut games = Vec::new();
    for sub in reg_subkeys(HKEY_LOCAL_MACHINE, base) {
        let key = format!("{base}\\{sub}");
        let Some(dir) = reg_string(HKEY_LOCAL_MACHINE, &key, "path").map(PathBuf::from) else { continue };
        if !dir.exists() {
            continue;
        }
        let name = reg_string(HKEY_LOCAL_MACHINE, &key, "gameName")
            .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        if not_a_game(&name) {
            continue;
        }
        games.push(LibraryGame { launcher: "GOG".into(), id: Some(sub), name, dir, poster: None, steam_appid: None });
    }
    games
}

pub fn ubisoft() -> Vec<LibraryGame> {
    let base = "SOFTWARE\\WOW6432Node\\Ubisoft\\Launcher\\Installs";
    let mut games = Vec::new();
    for sub in reg_subkeys(HKEY_LOCAL_MACHINE, base) {
        let Some(raw) = reg_string(HKEY_LOCAL_MACHINE, &format!("{base}\\{sub}"), "InstallDir") else { continue };
        let dir = PathBuf::from(raw.replace('/', "\\").trim_end_matches('\\'));
        if !dir.exists() {
            continue;
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if not_a_game(&name) {
            continue;
        }
        games.push(LibraryGame { launcher: "Ubisoft".into(), id: Some(sub), name, dir, poster: None, steam_appid: None });
    }
    games
}

/// A user-added folder can be one game or a library of game folders.
pub fn from_folder(root: &Path) -> Vec<LibraryGame> {
    let looks_like_game = |d: &Path| -> bool {
        fs::read_dir(d).map(|r| {
            r.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.ends_with(".exe") || n == "binaries" || n == "bin" || n == "x64" || n == "win64"
            })
        }).unwrap_or(false)
    };
    let mk = |d: PathBuf| LibraryGame {
        launcher: "Added".into(),
        id: None,
        name: d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        dir: d,
        poster: None,
        steam_appid: None,
    };
    if root.join("steamapps").join("common").is_dir() {
        return from_folder(&root.join("steamapps").join("common"));
    }
    let skip = Regex::new(r"(?i)^(_dlss5_backup|reshade-shaders|\$recycle\.bin|system volume information)$").unwrap();
    let mut children: Vec<LibraryGame> = fs::read_dir(root)
        .map(|r| {
            r.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter(|e| !skip.is_match(&e.file_name().to_string_lossy()))
                .map(|e| e.path())
                .filter(|p| looks_like_game(p) || fs::read_dir(p).map(|r| r.flatten().any(|c| c.path().is_dir())).unwrap_or(false))
                .map(mk)
                .collect()
        })
        .unwrap_or_default();
    if looks_like_game(root) || children.is_empty() {
        return vec![mk(root.to_path_buf())];
    }
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    children
}

/// Every launcher plus user-added folders and games, de-duplicated by path.
pub fn discover(manual_games: &[PathBuf], folders: &[PathBuf], excluded: &[PathBuf]) -> Vec<LibraryGame> {
    let mut all = Vec::new();
    all.extend(steam());
    all.extend(epic());
    all.extend(gog());
    all.extend(ubisoft());
    for f in folders {
        all.extend(from_folder(f));
    }
    for g in manual_games {
        if g.exists() {
            all.push(LibraryGame {
                launcher: "Added".into(),
                id: None,
                name: g.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                dir: g.clone(),
                poster: None,
                steam_appid: None,
            });
        }
    }
    let key = |p: &Path| p.to_string_lossy().to_lowercase().trim_end_matches('\\').to_string();
    let excluded: Vec<String> = excluded.iter().map(|p| key(p)).collect();
    let mut seen = Vec::new();
    all.retain(|g| {
        let k = key(&g.dir);
        if excluded.contains(&k) || seen.contains(&k) {
            return false;
        }
        seen.push(k);
        true
    });
    all
}
