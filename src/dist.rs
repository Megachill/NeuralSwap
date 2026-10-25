//! Distribution without a backend: the payload zip, the self-installing
//! setup executable (app + payload zip + trailer), and the `update.json`
//! manifest that GitHub Releases serves for auto-update.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Written after the zip appended to a setup exe: zip offset (u64 LE) + magic.
const MAGIC: &[u8; 8] = b"NSWPAYLD";

pub const SETUP_NAME: &str = "NeuralSwap-Setup.exe";
pub const APP_ASSET: &str = "NeuralSwap.exe";
pub const PAYLOAD_ASSET: &str = "NeuralSwap-payload.zip";
pub const MANIFEST_ASSET: &str = "update.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PayloadAsset {
    pub id: String,
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

/// `update.json`, attached to every GitHub release.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UpdateManifest {
    pub version: String,
    pub app: Asset,
    /// Absent for exe-only releases: the DLSS files are not published.
    #[serde(default)]
    pub payload: Option<PayloadAsset>,
    #[serde(default)]
    pub notes: String,
}

fn files_under(root: &Path) -> Vec<(String, PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(meta) = e.metadata() {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, p, meta.len()));
            }
        }
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

/// A cheap fingerprint of a payload folder (paths + sizes), so an update
/// only downloads the 250 MB payload when it actually changed.
pub fn payload_id(root: &Path) -> String {
    let mut h = Sha256::new();
    for (rel, _, size) in files_under(root) {
        h.update(rel.to_lowercase().as_bytes());
        h.update(size.to_le_bytes());
    }
    format!("{:x}", h.finalize())[..16].to_string()
}

pub fn payload_size(root: &Path) -> u64 {
    files_under(root).iter().map(|f| f.2).sum()
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn zip_payload(root: &Path, out: &Path, log: &mut dyn FnMut(String)) -> io::Result<()> {
    let mut zip = zip::ZipWriter::new(File::create(out)?);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let files = files_under(root);
    for (i, (rel, path, size)) in files.iter().enumerate() {
        zip.start_file(rel.as_str(), opts).map_err(io::Error::other)?;
        io::copy(&mut File::open(path)?, &mut zip)?;
        if *size > 4 << 20 {
            log(format!("  [{}/{}] {rel} ({} MB)", i + 1, files.len(), size >> 20));
        }
    }
    zip.finish().map_err(io::Error::other)?;
    Ok(())
}

fn write_setup(app_bytes: &[u8], zip_path: &Path, setup: &Path) -> io::Result<()> {
    let mut f = File::create(setup)?;
    f.write_all(app_bytes)?;
    io::copy(&mut File::open(zip_path)?, &mut f)?;
    f.write_all(&(app_bytes.len() as u64).to_le_bytes())?;
    f.write_all(MAGIC)?;
    Ok(())
}

/// Builds a release into `out_dir`: NeuralSwap.exe, NeuralSwap-Setup.exe and
/// update.json, plus NeuralSwap-payload.zip when `publish_payload`.
///
/// Without `publish_payload` the public setup carries no DLSS files (they
/// are imported after install), and a full setup for sharing privately goes
/// to `out_dir\private` — never upload that folder.
pub fn pack_release(payload_root: Option<&Path>, publish_payload: bool, out_dir: &Path, notes: &str, log: &mut dyn FnMut(String)) -> io::Result<UpdateManifest> {
    fs::create_dir_all(out_dir)?;
    let exe = std::env::current_exe()?;
    // A setup exe carries a payload; the plain app is what precedes it.
    let app_bytes = match appended_payload_offset(&exe)? {
        Some(off) => {
            let mut b = vec![0u8; off as usize];
            File::open(&exe)?.read_exact(&mut b)?;
            b
        }
        None => fs::read(&exe)?,
    };
    let app = out_dir.join(APP_ASSET);
    fs::write(&app, &app_bytes)?;
    log(format!("App: {} ({} MB)", app.display(), app_bytes.len() >> 20));

    let zip_dir = if publish_payload { out_dir.to_path_buf() } else { out_dir.join("private") };
    let mut payload = None;
    if let Some(root) = payload_root {
        fs::create_dir_all(&zip_dir)?;
        let zip_path = zip_dir.join(PAYLOAD_ASSET);
        log(format!("Compressing payload from {} …", root.display()));
        zip_payload(root, &zip_path, log)?;
        log(format!("Payload: {} ({} MB)", zip_path.display(), fs::metadata(&zip_path)?.len() >> 20));
        let full = zip_dir.join(if publish_payload { SETUP_NAME } else { "NeuralSwap-Setup-Full.exe" });
        write_setup(&app_bytes, &zip_path, &full)?;
        log(format!("Full setup: {} ({} MB)", full.display(), fs::metadata(&full)?.len() >> 20));
        if publish_payload {
            payload = Some(PayloadAsset {
                id: payload_id(root),
                name: PAYLOAD_ASSET.into(),
                sha256: sha256_file(&zip_path)?,
                size: fs::metadata(&zip_path)?.len(),
            });
        }
    } else if publish_payload {
        return Err(io::Error::other("no payload folder to publish"));
    }

    if !publish_payload {
        // An empty zip still makes the exe a setup, so it shows the installer.
        let empty = out_dir.join(".empty.zip");
        zip::ZipWriter::new(File::create(&empty)?).finish().map_err(io::Error::other)?;
        let setup = out_dir.join(SETUP_NAME);
        write_setup(&app_bytes, &empty, &setup)?;
        fs::remove_file(&empty)?;
        log(format!("Public setup (no DLSS files): {} ({} MB)", setup.display(), fs::metadata(&setup)?.len() >> 20));
    }

    let manifest = UpdateManifest {
        version: env!("CARGO_PKG_VERSION").to_string(),
        app: Asset { name: APP_ASSET.into(), sha256: sha256_file(&app)?, size: app_bytes.len() as u64 },
        payload,
        notes: notes.to_string(),
    };
    fs::write(out_dir.join(MANIFEST_ASSET), serde_json::to_string_pretty(&manifest)?)?;
    log(format!("Manifest: {}", out_dir.join(MANIFEST_ASSET).display()));
    Ok(manifest)
}

/// Number of files in the zip appended to a setup exe (0 for a public,
/// payload-less setup).
pub fn appended_entry_count(exe: &Path) -> usize {
    let Ok(Some(off)) = appended_payload_offset(exe) else { return 0 };
    let Ok(f) = File::open(exe) else { return 0 };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    zip::ZipArchive::new(Section { f, start: off, len: len - 16 - off, pos: 0 }).map(|z| z.len()).unwrap_or(0)
}

/// Where the appended payload zip starts, when this exe is a setup exe.
pub fn appended_payload_offset(exe: &Path) -> io::Result<Option<u64>> {
    let mut f = File::open(exe)?;
    let len = f.metadata()?.len();
    if len < 16 {
        return Ok(None);
    }
    f.seek(SeekFrom::End(-16))?;
    let mut tail = [0u8; 16];
    f.read_exact(&mut tail)?;
    if &tail[8..] != MAGIC {
        return Ok(None);
    }
    let off = u64::from_le_bytes(tail[..8].try_into().unwrap());
    Ok((off < len - 16).then_some(off))
}

/// A window onto part of a file, so the zip reader sees only the zip.
struct Section {
    f: File,
    start: u64,
    len: u64,
    pos: u64,
}

impl Read for Section {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos) as usize;
        let n = buf.len().min(left);
        if n == 0 {
            return Ok(0);
        }
        self.f.seek(SeekFrom::Start(self.start + self.pos))?;
        let r = self.f.read(&mut buf[..n])?;
        self.pos += r as u64;
        Ok(r)
    }
}

impl Seek for Section {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let new = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

/// Extracts a zip (a whole file, or the payload appended to a setup exe)
/// into `dest`. `progress` gets (bytes done, bytes total).
pub fn extract(zip_file: &Path, appended: bool, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> io::Result<()> {
    let f = File::open(zip_file)?;
    let total_len = f.metadata()?.len();
    let reader = if appended {
        let off = appended_payload_offset(zip_file)?.ok_or_else(|| io::Error::other("no payload in this executable"))?;
        Section { f, start: off, len: total_len - 16 - off, pos: 0 }
    } else {
        Section { f, start: 0, len: total_len, pos: 0 }
    };
    let mut zip = zip::ZipArchive::new(io::BufReader::with_capacity(1 << 20, reader)).map_err(io::Error::other)?;
    let total: u64 = (0..zip.len()).filter_map(|i| zip.by_index_raw(i).ok().map(|e| e.size())).sum();
    let mut done = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(io::Error::other)?;
        // enclosed_name refuses absolute paths and "..": nothing escapes dest.
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(d) = out.parent() {
            fs::create_dir_all(d)?;
        }
        let mut w = File::create(&out)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
    }
    Ok(())
}

/// Copies the plain app (without any appended payload) to `dest`.
pub fn write_app_only(exe: &Path, dest: &Path) -> io::Result<()> {
    match appended_payload_offset(exe)? {
        Some(off) => {
            let mut src = File::open(exe)?.take(off);
            let mut out = File::create(dest)?;
            io::copy(&mut src, &mut out)?;
            Ok(())
        }
        None => fs::copy(exe, dest).map(|_| ()),
    }
}
