//! Building the DLSS 5 payload without shipping it.
//!
//! *Official* components come straight from their publishers, pinned to an
//! exact release and SHA-256 checked before a byte is used: NVIDIA DLSS,
//! ReShade, DLSS5-Feeder, VORT shaders and the ReShade headers.
//!
//! The two *core* files have no official source — the DLSS 5 neural runtime
//! (`nvngx_dlssnr.dll`, a modified NVIDIA build) and the RenoDX DLSS 5
//! add-ons. They are imported from a zip or a link the user chooses; builds
//! whose hash is not one we know require an explicit "install anyway".

use crate::dist;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

enum Unpack {
    /// A single file, placed at this payload-relative path.
    File(&'static str),
    /// A zip: (path inside the archive, payload-relative destination).
    /// A trailing '/' maps a whole folder.
    Zip(&'static [(&'static str, &'static str)]),
}

pub struct Official {
    pub label: &'static str,
    pub version: &'static str,
    url: &'static str,
    sha256: &'static str,
    pub size: u64,
    unpack: Unpack,
}

#[cfg(test)]
const VORT: &str = "vort_Shaders-b410b9f0c0fbb83c8cb42164aaf1655fab386f4a/";

pub const OFFICIAL: &[Official] = &[
    Official {
        label: "NVIDIA DLSS Super Resolution",
        version: "310.9.16",
        url: "https://raw.githubusercontent.com/NVIDIA/DLSS/v310.9.16/lib/Windows_x86_64/rel/nvngx_dlss.dll",
        sha256: "d00b5b0ab29c3ca101b70c3a5ecb3a01b7ef33766a3383a28a18623083e6123e",
        size: 58_956_912,
        unpack: Unpack::File("streamline/nvngx_dlss.dll"),
    },
    Official {
        label: "NVIDIA DLSS licence",
        version: "310.9.16",
        url: "https://raw.githubusercontent.com/NVIDIA/DLSS/v310.9.16/LICENSE.txt",
        sha256: "d4216e39ebef5f9b50a6712ebb37beeb5379862a67733a9999c651f21592aaf0",
        size: 26_620,
        unpack: Unpack::File("streamline/nvngx_dlss.license.txt"),
    },
    Official {
        label: "ReShade (add-on build)",
        version: "6.8.0",
        url: "https://reshade.me/downloads/ReShade_Setup_6.8.0_Addon.exe",
        sha256: "afe4c8f13048306307983b8b3d41d5bf00a86820440b0e57dea10950e1176445",
        size: 4_318_424,
        unpack: Unpack::Zip(&[
            ("ReShade64.dll", "reshade-vulkan/ReShade64.dll"),
            ("ReShade64.json", "reshade-vulkan/ReShade64.json"),
            ("ReShade32.dll", "reshade-vulkan/ReShade32.dll"),
            ("ReShade32.json", "reshade-vulkan/ReShade32.json"),
        ]),
    },
    Official {
        label: "DLSS5-Feeder",
        version: "0.15.1",
        url: "https://github.com/jlrouzies-fr/DLSS5-Feeder/releases/download/v0.15.1/DLSS5-Feeder-0.15.1.zip",
        sha256: "2e44e81e691e75e532b9b7babc278a12615cb7f0fd9ef854da50e6ef17b272f4",
        size: 358_284,
        unpack: Unpack::Zip(&[
            ("host64/dlss5-feed-host64.exe", "feeder/dlss5-feed-host64.exe"),
            ("dlss5-feed.addon64", "feeder/dlss5-feed.addon64"),
            ("dlss5-feed.addon32", "feeder/dlss5-feed.addon32"),
            ("Verify-DLSS5Feeder.ps1", "feeder/Verify-DLSS5Feeder.ps1"),
            ("reshade-shaders/Shaders/DLSS5_Feed.fx", "feeder/reshade-shaders/Shaders/DLSS5_Feed.fx"),
            ("layer-x64/", "feeder/layer-x64/"),
            ("layer-x86/", "feeder/layer-x86/"),
        ]),
    },
    Official {
        label: "DLSS5-Feeder licence",
        version: "0.15.1",
        url: "https://raw.githubusercontent.com/jlrouzies-fr/DLSS5-Feeder/v0.15.1/LICENSE",
        sha256: "6562d5a5e3d7534711e34f4b34335f23f067acc839ae5274c1250bf5f4654b8b",
        size: 1_187,
        unpack: Unpack::File("feeder/licenses/DLSS5-Feeder-LICENSE.txt"),
    },
    Official {
        label: "VORT motion-vector shaders",
        version: "b410b9f",
        url: "https://codeload.github.com/vortigern11/vort_Shaders/zip/b410b9f0c0fbb83c8cb42164aaf1655fab386f4a",
        sha256: "231ba34a75556f9943e359559a89b0d0cc2caa322d9dcdee5630061bf9fe13b6",
        size: 4_211_561,
        unpack: Unpack::Zip(&[
            (concat!("vort_Shaders-b410b9f0c0fbb83c8cb42164aaf1655fab386f4a/", "Shaders/vort_Motion.fx"), "feeder/reshade-shaders/Shaders/vort_Motion.fx"),
            (concat!("vort_Shaders-b410b9f0c0fbb83c8cb42164aaf1655fab386f4a/", "Shaders/Includes/"), "feeder/reshade-shaders/Shaders/Includes/"),
            (concat!("vort_Shaders-b410b9f0c0fbb83c8cb42164aaf1655fab386f4a/", "Textures/"), "feeder/reshade-shaders/Textures/"),
            (concat!("vort_Shaders-b410b9f0c0fbb83c8cb42164aaf1655fab386f4a/", "LICENSE"), "feeder/licenses/VORT-LICENSE.txt"),
        ]),
    },
    Official {
        label: "ReShade shader header",
        version: "ee30868",
        url: "https://raw.githubusercontent.com/crosire/reshade-shaders/ee30868391d4ad103db60489820102d8fd40e3c1/Shaders/ReShade.fxh",
        sha256: "6dabfbbaf968c3871905d2ea17f96572ff7b1cec01310b5d0e5252b66b30174f",
        size: 4_250,
        unpack: Unpack::File("feeder/reshade-shaders/Shaders/ReShade.fxh"),
    },
    Official {
        label: "ReShade UI header",
        version: "ee30868",
        url: "https://raw.githubusercontent.com/crosire/reshade-shaders/ee30868391d4ad103db60489820102d8fd40e3c1/Shaders/ReShadeUI.fxh",
        sha256: "78adf672df47460297eb9fe6dd238d2aafa24510b52b84feb1a745dff70eb901",
        size: 9_930,
        unpack: Unpack::File("feeder/reshade-shaders/Shaders/ReShadeUI.fxh"),
    },
];

/// A core file: what it is called, where it goes, and the builds we know.
pub struct Core {
    pub name: &'static str,
    pub label: &'static str,
    pub required: bool,
    dests: &'static [&'static str],
    known: &'static [&'static str],
}

pub const CORE: &[Core] = &[
    Core {
        name: "nvngx_dlssnr.dll",
        label: "DLSS 5 neural runtime",
        required: true,
        dests: &["streamline/nvngx_dlssnr.dll"],
        known: &[
            // 310.8, first model (modified)
            "8270b350cd82de5ce89806872cdd6b6a9249b80836b91bbeb3573470744cc206",
            // 310.9.16, faster model (modified, signature removed, adds .nrcubin)
            "cdfa6c1ec784867182164c7836f324f087b5c9f374d0c9fe46f54999feabf3ed",
        ],
    },
    Core {
        name: "renodx-dlss5.addon64",
        label: "RenoDX DLSS 5 add-on",
        required: true,
        dests: &["renodx-dlss5.addon64", "feeder/host64/renodx-dlss5.addon64"],
        known: &["d5adf82eb44b065f4c590ac91fe824bab07afea0eb9f994bde936710c8593952"],
    },
    Core {
        name: "renodx-dlss.addon64",
        label: "RenoDX DLSS Tool (multipass)",
        required: false,
        dests: &["feeder/host64/renodx-dlss.addon64"],
        known: &[
            "1d855cf226857dce890cffbf7206ba9b6497ce1d471b217c1c8b44b6cd5d27e9",
            // ShortFuse 0.2026.1016.1704
            "a8fd34b531fd22c3dbd008ed8fd3cbfcca2fb451729e19cd1fc2e4c85b5068d4",
        ],
    },
];

pub const CORE_ZIP: &str = "NeuralSwap-dlss5-core.zip";

fn sha256_bytes(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

/// Whether `path` is the pinned build. Hashes are remembered per (path, size,
/// mtime), as the Home page asks every frame.
fn matches_pin(path: &Path, sha256: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::SystemTime;
    static SEEN: once_cell::sync::Lazy<Mutex<HashMap<(PathBuf, u64, SystemTime), String>>> = once_cell::sync::Lazy::new(Default::default);
    let Ok(meta) = fs::metadata(path) else { return false };
    let key = (path.to_path_buf(), meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
    if let Some(h) = SEEN.lock().unwrap().get(&key) {
        return h == sha256;
    }
    let Ok(h) = dist::sha256_file(path) else { return false };
    let ok = h == sha256;
    SEEN.lock().unwrap().insert(key, h);
    ok
}

/// Which official components are missing or differ from the pinned build
/// (an older payload may carry an earlier NVIDIA DLSS, for example).
pub fn official_missing(payload: &Path) -> Vec<&'static Official> {
    OFFICIAL
        .iter()
        .filter(|c| match &c.unpack {
            Unpack::File(rel) => !matches_pin(&payload.join(rel), c.sha256),
            Unpack::Zip(map) => map.iter().any(|(_, d)| if d.ends_with('/') { !payload.join(d).is_dir() } else { !payload.join(d).is_file() }),
        })
        .collect()
}

pub fn core_present(payload: &Path, c: &Core) -> bool {
    c.dests.iter().all(|d| payload.join(d).is_file())
}

fn cache_dir() -> PathBuf {
    let d = crate::state::data_dir().join("components");
    let _ = fs::create_dir_all(&d);
    d
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(300))).build().into()
}

/// Streams `url` to `dest` (via .part), reporting (done, total) bytes.
fn fetch(url: &str, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<String, String> {
    let mut res = agent()
        .get(url)
        .header("User-Agent", concat!("NeuralSwap/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let fingerprint = fingerprint_of(res.headers());
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
        progress(done, total);
    }
    drop(f);
    let _ = fs::remove_file(dest);
    fs::rename(&part, dest).map_err(|e| e.to_string())?;
    Ok(fingerprint)
}

/// What identifies a download without fetching it: ETag, Last-Modified and
/// size. Empty when the host sends none of them.
fn fingerprint_of(h: &ureq::http::HeaderMap) -> String {
    let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let size = h
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .map(String::from)
        .unwrap_or_else(|| get("content-length"));
    let parts = [get("etag"), get("last-modified"), size];
    if parts.iter().all(|p| p.is_empty()) { String::new() } else { parts.join("|") }
}

/// The share link's fingerprint, from its headers only (asks for one byte).
pub fn link_fingerprint(url: &str) -> Result<String, String> {
    let res = agent()
        .get(&direct_link(url))
        .header("User-Agent", concat!("NeuralSwap/", env!("CARGO_PKG_VERSION")))
        .header("Range", "bytes=0-0")
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    Ok(fingerprint_of(res.headers()))
}

pub fn is_known(name: &str, sha256: &str) -> bool {
    CORE.iter().any(|c| c.name == name && c.known.contains(&sha256))
}

/// Writes one core file to every place the payload needs it.
pub fn place_core(name: &str, bytes: &[u8], payload: &Path) -> io::Result<()> {
    let core = CORE.iter().find(|c| c.name == name).ok_or_else(|| io::Error::other(format!("not a core file: {name}")))?;
    for d in core.dests {
        write_file(payload, d, bytes)?;
    }
    Ok(())
}

fn write_file(payload: &Path, rel: &str, bytes: &[u8]) -> io::Result<()> {
    let dest = payload.join(rel.replace('/', "\\"));
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d)?;
    }
    crate::core::journal::clear_readonly(&dest);
    fs::write(dest, bytes)
}

/// Opens a zip, also when data precedes it (ReShade Setup is an exe with a
/// zip appended; the zip reader locates it from the end).
fn unzip_map(archive: &Path, map: &[(&str, &str)], payload: &Path) -> Result<usize, String> {
    let f = File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(f)).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut written = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let target = map.iter().find_map(|(src, dest)| {
            if src.ends_with('/') {
                name.strip_prefix(src).filter(|rest| !rest.is_empty() && !rest.contains("..")).map(|rest| format!("{dest}{rest}"))
            } else if name.eq_ignore_ascii_case(src) {
                Some(dest.to_string())
            } else {
                None
            }
        });
        let Some(target) = target else { continue };
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        write_file(payload, &target, &bytes).map_err(|e| e.to_string())?;
        written += 1;
    }
    let expected = map.iter().filter(|(s, _)| !s.ends_with('/')).count();
    let singles = map.iter().filter(|(s, _)| !s.ends_with('/')).all(|(_, d)| payload.join(d.replace('/', "\\")).is_file());
    if !singles || written < expected {
        return Err(format!("{} did not contain the expected files", archive.display()));
    }
    Ok(written)
}

/// Downloads every missing official component (pinned + SHA-256 verified)
/// into `payload`. `progress` gets (fraction, status).
pub fn download_official(payload: &Path, progress: &mut dyn FnMut(f32, String)) -> Result<(), String> {
    let todo: Vec<&Official> = official_missing(payload);
    let total: u64 = todo.iter().map(|c| c.size).sum::<u64>().max(1);
    let mut base = 0u64;
    for c in todo {
        let cached = cache_dir().join(c.sha256);
        let have = dist::sha256_file(&cached).map(|h| h == c.sha256).unwrap_or(false);
        if !have {
            fetch(c.url, &cached, &mut |d, _| {
                progress((base + d) as f32 / total as f32, format!("Downloading {} {}  {} / {} MB", c.label, c.version, d >> 20, c.size >> 20))
            })?;
            let got = dist::sha256_file(&cached).map_err(|e| e.to_string())?;
            if got != c.sha256 {
                let _ = fs::remove_file(&cached);
                return Err(format!("{} {} failed verification (SHA-256 mismatch) — nothing was installed", c.label, c.version));
            }
        }
        match &c.unpack {
            Unpack::File(rel) => {
                let bytes = fs::read(&cached).map_err(|e| e.to_string())?;
                write_file(payload, rel, &bytes).map_err(|e| e.to_string())?;
            }
            Unpack::Zip(map) => {
                unzip_map(&cached, map, payload)?;
            }
        }
        base += c.size;
        progress(base as f32 / total as f32, format!("{} {} ready", c.label, c.version));
    }
    write_sources_note(payload);
    Ok(())
}

fn write_sources_note(payload: &Path) {
    let mut text = String::from("Downloaded by NeuralSwap from the official sources, SHA-256 verified:\r\n");
    for c in OFFICIAL {
        text.push_str(&format!("{} {} - {}\r\n", c.label, c.version, c.url));
    }
    let _ = write_file(payload, "feeder/licenses/THIRD-PARTY-SOURCES.txt", text.as_bytes());
}

/// Before an install: fetch any missing official components into the
/// payload (pinned + verified), and return the refreshed payload.
pub fn ensure_official(payload: &crate::core::payload::Payload, log: &mut dyn FnMut(crate::core::Event)) -> crate::core::Result<crate::core::payload::Payload> {
    use crate::core::Event;
    if official_missing(&payload.root).is_empty() {
        return Ok(payload.clone());
    }
    log(Event::info(crate::i18n::t("Downloading the official components first (NVIDIA DLSS, ReShade, Feeder)…")));
    download_official(&payload.root, &mut |_, _| {}).map_err(|e| crate::core::fail("errComponentDownload", e))?;
    log(Event::ok(crate::i18n::t("Official components downloaded and SHA-256 verified.")));
    Ok(crate::core::payload::scan_payload(&payload.root).unwrap_or_else(|| payload.clone()))
}

/// Turns common share links into direct downloads.
pub fn direct_link(url: &str) -> String {
    let u = url.trim();
    if u.contains("dropbox.com") {
        return if u.contains("dl=0") { u.replace("dl=0", "dl=1") } else if u.contains('?') { format!("{u}&dl=1") } else { format!("{u}?dl=1") };
    }
    if u.contains("drive.google.com") {
        let id = u
            .split("/d/")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .or_else(|| u.split("id=").nth(1).and_then(|s| s.split('&').next()));
        if let Some(id) = id {
            return format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t");
        }
    }
    if (u.contains("1drv.ms") || u.contains("onedrive.live.com") || u.contains("sharepoint.com")) && !u.contains("download=1") {
        return if u.contains('?') { format!("{u}&download=1") } else { format!("{u}?download=1") };
    }
    u.to_string()
}

pub struct CoreFound {
    pub name: &'static str,
    pub known: bool,
}

/// Downloads a core zip from a link into the component cache.
pub fn download_link(url: &str, progress: &mut dyn FnMut(f32, String)) -> Result<(PathBuf, String), String> {
    let dest = cache_dir().join("core-download.zip");
    let fingerprint = fetch(&direct_link(url), &dest, &mut |d, t| {
        progress(if t > 0 { d as f32 / t as f32 } else { 0.0 }, format!("Downloading DLSS 5 core files  {} MB", d >> 20))
    })?;
    let mut magic = [0u8; 2];
    File::open(&dest).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| e.to_string())?;
    if &magic != b"PK" {
        let _ = fs::remove_file(&dest);
        return Err("the link did not return a zip file (for Google Drive / OneDrive, make sure the link is shared with \"anyone with the link\")".into());
    }
    Ok((dest, fingerprint))
}

/// Copies the core files found anywhere in `zip_path` into `payload`.
/// Unknown builds are refused unless `allow_unknown`; Err lists them.
pub fn import_core(zip_path: &Path, payload: &Path, allow_unknown: bool, expected_sha: Option<&str>) -> Result<Vec<CoreFound>, String> {
    if let Some(want) = expected_sha.map(str::trim).filter(|s| !s.is_empty()) {
        let got = dist::sha256_file(zip_path).map_err(|e| e.to_string())?;
        if !got.eq_ignore_ascii_case(want) {
            return Err(format!("the zip's SHA-256 is {got}, not the one you entered — nothing was installed"));
        }
    }
    let f = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(f)).map_err(|e| e.to_string())?;
    let mut found: Vec<(&'static Core, Vec<u8>, bool)> = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let file = entry.name().replace('\\', "/").rsplit('/').next().unwrap_or("").to_lowercase();
        let Some(core) = CORE.iter().find(|c| c.name == file) else { continue };
        if found.iter().any(|(c, _, _)| c.name == core.name) {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let known = core.known.contains(&sha256_bytes(&bytes).as_str());
        found.push((core, bytes, known));
    }
    if found.is_empty() {
        return Err("no DLSS 5 core files in that zip (looked for nvngx_dlssnr.dll and renodx-dlss5.addon64)".into());
    }
    let unknown: Vec<&str> = found.iter().filter(|(_, _, k)| !k).map(|(c, _, _)| c.name).collect();
    if !unknown.is_empty() && !allow_unknown {
        return Err(format!("unrecognised build of {} — only install files from someone you trust", unknown.join(", ")));
    }
    for (core, bytes, _) in &found {
        place_core(core.name, bytes, payload).map_err(|e| e.to_string())?;
    }
    Ok(found.into_iter().map(|(c, _, k)| CoreFound { name: c.name, known: k }).collect())
}

/// The small private zip of core files, for sharing alongside a payload-less
/// release.
pub fn pack_core(payload: &Path, out: &Path) -> io::Result<()> {
    let mut z = zip::ZipWriter::new(File::create(out)?);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    for c in CORE {
        let src = payload.join(c.dests[0].replace('/', "\\"));
        if src.is_file() {
            z.start_file(c.name, opts).map_err(io::Error::other)?;
            io::copy(&mut File::open(src)?, &mut z)?;
        }
    }
    z.finish().map_err(io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_official_file_counts_as_missing() {
        let dir = std::env::temp_dir().join(format!("ns-pin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let header = OFFICIAL.iter().find(|c| c.label == "ReShade shader header").unwrap();
        let Unpack::File(rel) = header.unpack else { unreachable!() };
        write_file(&dir, rel, b"an older header").unwrap();
        assert!(official_missing(&dir).iter().any(|c| c.label == header.label));
        assert!(!matches_pin(&dir.join(rel), header.sha256));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn share_links_become_direct() {
        assert_eq!(direct_link("https://www.dropbox.com/s/abc/x.zip?dl=0"), "https://www.dropbox.com/s/abc/x.zip?dl=1");
        assert_eq!(
            direct_link("https://drive.google.com/file/d/1AbC/view?usp=sharing"),
            "https://drive.usercontent.google.com/download?id=1AbC&export=download&confirm=t"
        );
        assert_eq!(direct_link("https://1drv.ms/u/s!abc"), "https://1drv.ms/u/s!abc?download=1");
        assert_eq!(direct_link("https://example.com/core.zip"), "https://example.com/core.zip");
    }

    #[test]
    fn vort_prefix_matches_constant() {
        assert!(OFFICIAL.iter().any(|c| matches!(&c.unpack, Unpack::Zip(m) if m.iter().any(|(s, _)| s.starts_with(VORT)))));
    }
}
