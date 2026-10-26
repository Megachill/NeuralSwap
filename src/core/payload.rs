//! The DLSS 5 payload: the neural-rendering runtime, the RenoDX add-ons, the
//! ReShade add-on builds and the DLSS5-Feeder files. It ships as a folder
//! beside the executable rather than inside it.

use super::pe;
use super::scan::{DLSS_FILE, STREAMLINE_FILE};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// One verified Feeder release; never mix host protocol versions.
pub const FEEDER_VERSION: &str = "0.15.1";
const FEEDER_HASHES: &[(&str, &str)] = &[
    ("dlss5-feed.addon32", "fb69357075cba536b42f2e693992fde0c1775058b1fd9f10fbdf1e7eba3443e7"),
    ("dlss5-feed.addon64", "3afc8efb5f516e94a2a068b2e90eaed360d1e30ca2c29b623e0cfadc1f05d50d"),
    ("dlss5-feed-host64.exe", "034e6cdf382e6ea9164afd63c5b8a8aa0312894cb342593bc933e9fa435f8d02"),
    ("reshade-shaders/Shaders/DLSS5_Feed.fx", "cdac08a721b14b97187dd86c5b5bead157c9063d7ee859a0f131a8ee791695f1"),
];

#[derive(Clone, Debug, Serialize)]
pub struct PayloadFile {
    pub name: String,
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Feeder {
    pub dir: PathBuf,
    pub addon64: PathBuf,
    pub host_addon: PathBuf,
    pub shader_root: PathBuf,
    pub release_verified: bool,
    pub ok64: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Payload {
    pub root: PathBuf,
    pub streamline_dir: PathBuf,
    pub files: Vec<PayloadFile>,
    /// The native route's RenoDX consumer (renodx-dlss5.addon64).
    pub addon: Option<PathBuf>,
    /// ShortFuse's RenoDX DLSS Tool (multipass) build, when shipped.
    pub multipass_addon: Option<PathBuf>,
    /// The in-game DLSS 5 Lab overlay add-on, when shipped.
    pub overlay_addon: Option<PathBuf>,
    pub reshade64: Option<PathBuf>,
    pub reshade32: Option<PathBuf>,
    pub reshade_setup: Option<PathBuf>,
    pub feeder: Feeder,
    pub dlss_version: Option<String>,
    pub nr_version: Option<String>,
    pub reshade_version: Option<String>,
}

impl Payload {
    pub fn has_neural_rendering(&self) -> bool {
        self.file("nvngx_dlssnr.dll").is_some()
    }

    pub fn file(&self, name: &str) -> Option<&PayloadFile> {
        self.files.iter().find(|f| f.name.eq_ignore_ascii_case(name))
    }
}

fn existing(p: PathBuf) -> Option<PathBuf> {
    p.is_file().then_some(p)
}

fn sha256(file: &Path) -> Option<String> {
    let bytes = fs::read(file).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Validates a folder holding the payload (streamline\ plus add-ons).
pub fn scan_payload(root: &Path) -> Option<Payload> {
    let streamline_dir = if root.join("streamline").is_dir() { root.join("streamline") } else { root.to_path_buf() };
    let mut files: Vec<PayloadFile> = fs::read_dir(&streamline_dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (DLSS_FILE.is_match(&name) || STREAMLINE_FILE.is_match(&name)).then(|| PayloadFile {
                version: pe::file_version(&e.path()),
                path: e.path(),
                name,
            })
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let feeder_dir = root.join("feeder");
    // The native consumer is chosen by name: the multipass build
    // (renodx-dlss.addon64) sorts ahead of it and has a route of its own.
    let mut addon = None;
    for dir in [root, streamline_dir.as_path()] {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let names: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| {
                let l = n.to_lowercase();
                l.ends_with(".addon64") && l != "renodx-dlss.addon64" && !l.starts_with("dlss5-lab-overlay")
            })
            .collect();
        if let Some(found) = names.iter().find(|n| n.eq_ignore_ascii_case("renodx-dlss5.addon64")).or(names.first()) {
            addon = Some(dir.join(found));
            break;
        }
    }
    let host_addon = feeder_dir.join("host64").join("renodx-dlss5.addon64");
    if addon.is_none() && host_addon.is_file() {
        addon = Some(host_addon.clone());
    }

    let shader_root = feeder_dir.join("reshade-shaders");
    let release_verified = FEEDER_HASHES
        .iter()
        .all(|(rel, hash)| sha256(&feeder_dir.join(rel)).as_deref() == Some(*hash));
    let addon64 = feeder_dir.join("dlss5-feed.addon64");
    let ok64 = release_verified
        && [
            addon64.clone(),
            host_addon.clone(),
            shader_root.join("Shaders").join("DLSS5_Feed.fx"),
            shader_root.join("Shaders").join("vort_Motion.fx"),
            shader_root.join("Shaders").join("ReShade.fxh"),
            shader_root.join("Shaders").join("ReShadeUI.fxh"),
        ]
        .iter()
        .all(|p| p.is_file());

    let reshade_dir = root.join("reshade-vulkan");
    let reshade64 = existing(reshade_dir.join("ReShade64.dll"));
    let reshade32 = existing(reshade_dir.join("ReShade32.dll"));
    let reshade_setup = fs::read_dir(root).ok().and_then(|r| {
        r.flatten()
            .map(|e| e.path())
            .find(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                n.starts_with("reshade_setup_") && n.ends_with("_addon.exe")
            })
    });
    let reshade_version = reshade64.as_deref().and_then(pe::file_version);
    let overlay_addon = existing(root.join("dlss5-lab-overlay.addon64"))
        .or_else(|| root.parent().and_then(|p| existing(p.join("overlay").join("dlss5-lab-overlay.addon64"))));

    let find_version = |n: &str| files.iter().find(|f| f.name.eq_ignore_ascii_case(n)).and_then(|f| f.version.clone());
    Some(Payload {
        dlss_version: find_version("nvngx_dlss.dll"),
        nr_version: find_version("nvngx_dlssnr.dll"),
        root: root.to_path_buf(),
        streamline_dir,
        addon,
        multipass_addon: existing(feeder_dir.join("host64").join("renodx-dlss.addon64")),
        overlay_addon,
        reshade64,
        reshade32,
        reshade_setup,
        reshade_version,
        feeder: Feeder { dir: feeder_dir, addon64, host_addon, shader_root, release_verified, ok64 },
        files,
    })
}

/// Folders worth trying, in order: an explicit setting, a `payload` folder
/// beside the executable or any of its parents (covers `target\release`),
/// then an installed copy of the original Electron app.
pub fn candidates(setting: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(s) = setting {
        out.push(s.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..5 {
            let Some(d) = dir else { break };
            out.push(d.join("payload"));
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push(Path::new(&local).join("Programs").join("DLSS 5 Swapper").join("resources").join("payload"));
    }
    out
}

impl Payload {
    /// Everything an install needs: the neural runtime, the RenoDX add-on
    /// and the ReShade add-on build that loads it.
    pub fn complete(&self) -> bool {
        self.has_neural_rendering() && self.addon.is_some() && self.reshade64.is_some()
    }
}

/// The first complete payload, else the first partial one.
pub fn locate(setting: Option<&Path>) -> Option<Payload> {
    let found: Vec<Payload> = candidates(setting).into_iter().filter_map(|dir| scan_payload(&dir)).collect();
    let pos = found.iter().position(|p| p.complete()).unwrap_or(0);
    found.into_iter().nth(pos)
}
