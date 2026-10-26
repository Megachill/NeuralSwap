//! Finding DLSS 5 core files that are already on this PC — left there by
//! other DLSS 5 tools, by an earlier NeuralSwap/DLSS 5 Swapper install in a
//! game folder, or shipped by a game (NBA 2K27 carries NVIDIA's own
//! nvngx_dlssnr.dll) — and telling which DLSS 5 model a file is.

use crate::components::{self, CORE};
use crate::core::pe;
use crate::library::reg_string;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

/// The neural-rendering model a nvngx_dlssnr.dll carries, by version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// 310.8: the first DLSS 5 model.
    First,
    /// 310.9 and newer: smaller, less VRAM, faster neural rendering.
    Faster,
    Unknown,
}

pub fn model_of(version: Option<&str>) -> Model {
    match version {
        None => Model::Unknown,
        Some(v) if pe::compare_versions(Some(v), Some("310.9.0")) >= 0 => Model::Faster,
        Some(v) if pe::compare_versions(Some(v), Some("310.8.0")) >= 0 => Model::First,
        Some(_) => Model::Unknown,
    }
}

/// True for an intact NVIDIA-signed file: NVIDIA's original build, which
/// only creates the neural feature on RTX 50. Modified builds for RTX 20–40
/// fail this check because patching breaks the signature.
pub fn nvidia_signed(file: &Path) -> bool {
    use windows_sys::Win32::Security::WinTrust::*;
    let path: Vec<u16> = file.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: path.as_ptr(), ..Default::default() };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        dwStateAction: WTD_STATEACTION_VERIFY,
        // Never go online for a check: cached revocation data only.
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    data.Anonymous.pFile = &mut info;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let ok = unsafe { WinVerifyTrust(std::ptr::null_mut(), &mut action, &mut data as *mut _ as *mut _) } == 0;
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe { WinVerifyTrust(std::ptr::null_mut(), &mut action, &mut data as *mut _ as *mut _) };
    ok && pe::version_mentions(file, "NVIDIA")
}

#[derive(Clone, Debug)]
pub struct CoreFile {
    pub name: &'static str,
    pub path: PathBuf,
    pub version: Option<String>,
    pub sha256: String,
    pub known: bool,
}

#[derive(Clone, Debug)]
pub struct CoreSource {
    /// Where it was found, for people: "NBA 2K27", "DLSS 5 Swapper", a folder.
    pub label: String,
    pub dir: PathBuf,
    pub files: Vec<CoreFile>,
    /// For the neural runtime: NVIDIA's intact signature (RTX 50 only).
    pub signed: bool,
}

impl CoreSource {
    pub fn runtime(&self) -> Option<&CoreFile> {
        self.files.iter().find(|f| f.name == "nvngx_dlssnr.dll")
    }
    pub fn model(&self) -> Model {
        model_of(self.runtime().and_then(|f| f.version.as_deref()))
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// InstallLocation of installed apps whose name suggests a DLSS 5 tool.
fn installed_tools() -> Vec<(String, PathBuf)> {
    let pattern = regex::Regex::new(r"(?i)dlss|magpie|renodx|optiscaler|neural").unwrap();
    let mut out = Vec::new();
    for (root, base) in [
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (HKEY_LOCAL_MACHINE, "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ] {
        let k = wide(base);
        let mut h: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, k.as_ptr(), 0, KEY_READ, &mut h) } != 0 {
            continue;
        }
        let mut i = 0;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let rc = unsafe { RegEnumKeyExW(h, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            if rc != 0 {
                break;
            }
            i += 1;
            let sub = format!("{base}\\{}", String::from_utf16_lossy(&name[..len as usize]));
            let Some(display) = reg_string(root, &sub, "DisplayName") else { continue };
            if display == "NeuralSwap" || !pattern.is_match(&display) {
                continue;
            }
            // Many installers leave InstallLocation empty; the folder of the
            // icon or the uninstaller says the same thing.
            let from_file = |value: &str| {
                reg_string(root, &sub, value).and_then(|v| {
                    let path = v.trim().trim_start_matches('"').split(['"', ',']).next().unwrap_or("").trim().to_string();
                    Path::new(&path).parent().map(Path::to_path_buf)
                })
            };
            let loc = reg_string(root, &sub, "InstallLocation")
                .filter(|v| !v.trim().is_empty())
                .map(PathBuf::from)
                .or_else(|| from_file("DisplayIcon"))
                .or_else(|| from_file("UninstallString"))
                .filter(|p| p.is_dir());
            if let Some(loc) = loc {
                out.push((display, loc));
            }
        }
        unsafe { RegCloseKey(h) };
    }
    out
}

/// Every core file under `dir` (bounded breadth-first walk).
fn collect(dir: &Path, max_depth: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    let mut queue = VecDeque::from([(dir.to_path_buf(), 0usize)]);
    while let Some((d, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().to_lowercase();
            if ft.is_dir() {
                if depth < max_depth && !matches!(name.as_str(), "_dlss5_backup" | "node_modules" | ".git" | "content" | "paks") {
                    queue.push_back((e.path(), depth + 1));
                }
            } else if CORE.iter().any(|c| c.name == name) {
                out.push(e.path());
            }
        }
    }
}

/// Searches the PC for DLSS 5 core files. `games` are (name, folder) of
/// library games, `exclude` is NeuralSwap's own payload.
pub fn find(games: &[(String, PathBuf)], magpie: Option<&Path>, exclude: Option<&Path>) -> Vec<CoreSource> {
    let mut places: Vec<(String, PathBuf, usize)> = Vec::new();
    for (name, dir) in installed_tools() {
        places.push((name, dir, 6));
    }
    if let Some(m) = magpie.and_then(Path::parent) {
        places.push(("Magpie".into(), m.to_path_buf(), 6));
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        for d in ["Downloads", "Desktop", "Documents"] {
            places.push((d.to_string(), Path::new(&profile).join(d), 3));
        }
    }
    for (name, dir) in games {
        places.push((name.clone(), dir.clone(), 6));
    }

    let excluded = exclude.map(|e| e.to_string_lossy().to_lowercase());
    let mut by_dir: Vec<(String, PathBuf, Vec<PathBuf>)> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    for (label, dir, depth) in places {
        let mut budget = 4000;
        let mut found = Vec::new();
        collect(&dir, depth, &mut budget, &mut found);
        for f in found {
            let key = f.to_string_lossy().to_lowercase();
            if excluded.as_deref().map(|e| key.starts_with(e)).unwrap_or(false) || !seen_paths.insert(key) {
                continue;
            }
            // A payload keeps the runtime in streamline\ and the add-ons a
            // level up; group both under the payload root.
            let parent = f.parent().unwrap_or(&dir).to_path_buf();
            let home = if parent.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("streamline")).unwrap_or(false) {
                parent.parent().unwrap_or(&parent).to_path_buf()
            } else if parent.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("host64")).unwrap_or(false) {
                parent.parent().and_then(Path::parent).unwrap_or(&parent).to_path_buf()
            } else {
                parent
            };
            match by_dir.iter_mut().find(|(_, d, _)| *d == home) {
                Some((_, _, list)) => list.push(f),
                None => by_dir.push((label.clone(), home, vec![f])),
            }
        }
    }

    let mut sources: Vec<CoreSource> = Vec::new();
    let mut seen_sets: HashSet<String> = HashSet::new();
    for (label, dir, paths) in by_dir {
        let mut files: Vec<CoreFile> = Vec::new();
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let Some(core) = CORE.iter().find(|c| c.name == name) else { continue };
            if files.iter().any(|f| f.name == core.name) {
                continue;
            }
            let Ok(sha) = crate::dist::sha256_file(&p) else { continue };
            files.push(CoreFile { name: core.name, version: pe::file_version(&p), known: components::is_known(core.name, &sha), sha256: sha, path: p });
        }
        if files.is_empty() {
            continue;
        }
        // The same build left in many game folders is shown once.
        let mut set: Vec<&str> = files.iter().map(|f| f.sha256.as_str()).collect();
        set.sort();
        if !seen_sets.insert(set.join("+")) {
            continue;
        }
        let signed = files.iter().find(|f| f.name == "nvngx_dlssnr.dll").map(|f| nvidia_signed(&f.path)).unwrap_or(false);
        sources.push(CoreSource { label, dir, files, signed });
    }
    // Faster model first, then the most complete sets.
    sources.sort_by_key(|s| (s.model() != Model::Faster, std::cmp::Reverse(s.files.len())));
    sources
}

/// Copies a found source's files into the payload.
pub fn use_source(src: &CoreSource, payload: &Path) -> Result<(), String> {
    for f in &src.files {
        let bytes = fs::read(&f.path).map_err(|e| format!("{}: {e}", f.path.display()))?;
        components::place_core(f.name, &bytes, payload).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_by_version() {
        assert_eq!(model_of(Some("310.8.0.0")), Model::First);
        assert_eq!(model_of(Some("310.9.16.0")), Model::Faster);
        assert_eq!(model_of(Some("311.0.1.0")), Model::Faster);
        assert_eq!(model_of(Some("2.5.0")), Model::Unknown);
        assert_eq!(model_of(None), Model::Unknown);
    }

    #[test]
    fn groups_a_payload_layout_under_one_source() {
        let root = std::env::temp_dir().join(format!("ns-corefind-{}", std::process::id()));
        let payload = root.join("SomeTool").join("payload");
        fs::create_dir_all(payload.join("streamline")).unwrap();
        fs::write(payload.join("streamline").join("nvngx_dlssnr.dll"), b"runtime").unwrap();
        fs::write(payload.join("renodx-dlss5.addon64"), b"addon").unwrap();
        let found = find(&[("Test".into(), root.clone())], None, None);
        let _ = fs::remove_dir_all(&root);
        let mine: Vec<_> = found.iter().filter(|s| s.dir == payload).collect();
        assert_eq!(mine.len(), 1, "{found:?}");
        assert_eq!(mine[0].files.len(), 2);
        assert!(mine[0].files.iter().all(|f| !f.known));
    }
}
