//! Magpie (SAOG0721's experimental fork): the anti-cheat-safe route. It
//! captures the game window from outside and runs DLSS 5 neural rendering on
//! the captured image, so nothing is loaded into the game process.
//!
//! NeuralSwap never bundles it (GPL-3.0 vs our MIT): it finds an existing
//! copy, launches it, and links to the fork's official releases.

use crate::core::{guards, pe};
use std::fs;
use std::path::{Path, PathBuf};

pub const RELEASES_URL: &str = "https://github.com/SAOG0721/Magpie/releases/latest";
pub const REPO_URL: &str = "https://github.com/SAOG0721/Magpie";

#[derive(Clone, Debug)]
pub struct Magpie {
    pub exe: PathBuf,
    pub version: Option<String>,
}

fn is_magpie(exe: &Path) -> bool {
    exe.is_file()
        && exe.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("Magpie.exe")).unwrap_or(false)
        && pe::bitness(exe).is_some()
}

fn found(exe: PathBuf) -> Magpie {
    Magpie { version: pe::file_version(&exe), exe }
}

/// Magpie.exe up to `depth` folders below `dir` (portable zips extract to
/// e.g. Downloads\Magpie-Experimental-x64\Magpie.exe).
fn search(dir: &Path, depth: usize, budget: &mut usize) -> Option<PathBuf> {
    let direct = dir.join("Magpie.exe");
    if is_magpie(&direct) {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let mut subdirs: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    // Folders that look like Magpie first; the rest only while budget lasts.
    subdirs.sort_by_key(|p| !p.file_name().map(|n| n.to_string_lossy().to_lowercase().contains("magpie")).unwrap_or(false));
    for sub in subdirs {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if let Some(hit) = search(&sub, depth - 1, budget) {
            return Some(hit);
        }
    }
    None
}

/// The chosen path, a running Magpie, or a copy in the usual places.
pub fn detect(setting: Option<&Path>) -> Option<Magpie> {
    if let Some(p) = setting.filter(|p| is_magpie(p)) {
        return Some(found(p.to_path_buf()));
    }
    if let Some(p) = guards::processes()
        .into_iter()
        .filter(|p| p.name.eq_ignore_ascii_case("Magpie.exe"))
        .find_map(|p| p.path.map(PathBuf::from))
        .filter(|p| is_magpie(p))
    {
        return Some(found(p));
    }
    let env = |k: &str| std::env::var(k).ok().map(PathBuf::from);
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let shallow: Vec<PathBuf> = [
        env("LOCALAPPDATA").map(|d| d.join("Programs").join("Magpie")),
        env("ProgramFiles").map(|d| d.join("Magpie")),
        Some(PathBuf::from(format!("{drive}\\Magpie"))),
        Some(PathBuf::from(format!("{drive}\\Tools\\Magpie"))),
    ]
    .into_iter()
    .flatten()
    .collect();
    for d in shallow {
        let mut budget = 20;
        if let Some(hit) = search(&d, 1, &mut budget) {
            return Some(found(hit));
        }
    }
    let profile = env("USERPROFILE")?;
    for d in ["Downloads", "Desktop", "Documents", "OneDrive\\Desktop", "OneDrive\\Documents"] {
        let mut budget = 60;
        if let Some(hit) = search(&profile.join(d), 2, &mut budget) {
            return Some(found(hit));
        }
    }
    None
}

pub fn launch(m: &Magpie) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(&m.exe);
    if let Some(dir) = m.exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(|_| ())
}

pub fn is_running() -> bool {
    guards::processes().iter().any(|p| p.name.eq_ignore_ascii_case("Magpie.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_portable_copy_and_an_explicit_path() {
        let root = std::env::temp_dir().join(format!("ns-magpie-test-{}", std::process::id()));
        let nested = root.join("Magpie-Experimental-x64");
        fs::create_dir_all(&nested).unwrap();
        // Any real PE file stands in for Magpie.exe.
        fs::copy(std::env::current_exe().unwrap(), nested.join("Magpie.exe")).unwrap();
        let mut budget = 10;
        assert_eq!(search(&root, 2, &mut budget), Some(nested.join("Magpie.exe")));
        assert!(detect(Some(&nested.join("Magpie.exe"))).is_some());
        // A non-PE file with the right name is not Magpie.
        fs::write(root.join("Magpie.exe"), b"not a program").unwrap();
        assert!(!is_magpie(&root.join("Magpie.exe")));
        let _ = fs::remove_dir_all(&root);
    }
}
