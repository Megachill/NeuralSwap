//! A write-ahead, file-level journal. Never snapshots or recursively deletes
//! a whole game: an interrupted install restores exactly the files that were
//! present immediately before it, including user-tuned settings.
//!
//! The on-disk layout matches the original DLSS 5 Swapper, so either tool can
//! recover the other's interrupted install.

use super::scan::BACKUP_DIR;
use super::{fail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JournalFile {
    pub rel: String,
    pub existed: bool,
    pub snapshot: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JournalState {
    pub version: u32,
    pub folder: String,
    pub files: Vec<JournalFile>,
    pub dirs: Vec<String>,
}

pub fn pending_path(game_dir: &Path) -> PathBuf {
    game_dir.join(BACKUP_DIR).join("pending-switch.json")
}

fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn key(p: &Path) -> String {
    lexical_normalize(p).to_string_lossy().to_lowercase().trim_end_matches('\\').to_string()
}

/// Resolves `rel` under `root`, refusing absolute paths, drive letters,
/// escapes and any symlink/junction between the target and the root.
pub fn safe_path(root: &Path, rel: &str) -> Result<PathBuf> {
    let unsafe_target = || fail("errUnsafeTarget", rel);
    if rel.is_empty() || rel.contains(':') || Path::new(rel).is_absolute() || rel.starts_with('\\') || rel.starts_with('/') {
        return Err(unsafe_target());
    }
    let root_n = lexical_normalize(root);
    let dest = lexical_normalize(&root_n.join(rel.replace('/', "\\")));
    let root_key = key(&root_n);
    let dest_key = key(&dest);
    if dest_key == root_key || !dest_key.starts_with(&(root_key.clone() + "\\")) {
        return Err(unsafe_target());
    }
    let mut item = dest.clone();
    loop {
        if let Ok(meta) = fs::symlink_metadata(&item) {
            if meta.file_type().is_symlink() {
                return Err(unsafe_target());
            }
        }
        if key(&item) == root_key {
            break;
        }
        match item.parent() {
            Some(p) => item = p.to_path_buf(),
            None => break,
        }
    }
    Ok(dest)
}

pub fn rel_from(root: &Path, target: &Path) -> String {
    let r = lexical_normalize(root);
    let t = lexical_normalize(target);
    let rs = r.to_string_lossy().to_string();
    let ts = t.to_string_lossy().to_string();
    if ts.to_lowercase().starts_with(&(rs.to_lowercase().trim_end_matches('\\').to_string() + "\\")) {
        ts[rs.trim_end_matches('\\').len() + 1..].to_string()
    } else {
        ts
    }
}

pub fn atomic_json<T: Serialize>(file: &Path, data: &T) -> Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = PathBuf::from(format!("{}.tmp", file.display()));
    fs::write(&temp, serde_json::to_string_pretty(data)?)?;
    fs::rename(&temp, file)?;
    Ok(())
}

impl JournalState {
    pub fn new() -> Self {
        JournalState {
            version: 1,
            folder: format!("{BACKUP_DIR}/.transactions/{}", uuid::Uuid::new_v4()),
            files: Vec::new(),
            dirs: Vec::new(),
        }
    }

    /// Records the bytes of `target` (or its absence) before the first write.
    pub fn capture(&mut self, game_dir: &Path, target: &Path) -> Result<()> {
        let rel = rel_from(game_dir, target);
        safe_path(game_dir, &rel)?;
        if self.files.iter().any(|f| f.rel.eq_ignore_ascii_case(&rel)) {
            return Ok(());
        }
        let mut parent = target.parent().map(Path::to_path_buf);
        while let Some(p) = parent {
            if key(&p) == key(game_dir) {
                break;
            }
            let dir = rel_from(game_dir, &p);
            if !p.exists() && !self.dirs.contains(&dir) {
                self.dirs.push(dir);
            }
            parent = p.parent().map(Path::to_path_buf);
        }
        let existed = target.exists();
        if existed && !target.is_file() {
            return Err(fail("errUnsafeTarget", &rel));
        }
        let snapshot = format!("{}/{}.bin", self.folder, self.files.len());
        if existed {
            let copy = safe_path(game_dir, &snapshot)?;
            if let Some(d) = copy.parent() {
                fs::create_dir_all(d)?;
            }
            fs::copy(target, &copy)?;
        }
        self.files.push(JournalFile { rel, existed, snapshot });
        atomic_json(&pending_path(game_dir), self)
    }

    fn cleanup(&self, game_dir: &Path) {
        for item in &self.files {
            if let Ok(f) = safe_path(game_dir, &item.snapshot) {
                let _ = fs::remove_file(f);
            }
        }
        if let Ok(d) = safe_path(game_dir, &self.folder) {
            let _ = fs::remove_dir(d);
        }
        if let Ok(d) = safe_path(game_dir, &format!("{BACKUP_DIR}/.transactions")) {
            let _ = fs::remove_dir(d);
        }
    }
}

pub fn begin(game_dir: &Path) -> Result<JournalState> {
    if pending_path(game_dir).exists() {
        return Err(fail("errBackendRecovery", ""));
    }
    safe_path(game_dir, &format!("{BACKUP_DIR}/pending-switch.json"))?;
    let mut state = JournalState::new();
    atomic_json(&pending_path(game_dir), &state)?;
    state.capture(game_dir, &game_dir.join(BACKUP_DIR).join("manifest.json"))?;
    Ok(state)
}

pub fn commit(game_dir: &Path, state: &JournalState) -> Result<()> {
    fs::remove_file(pending_path(game_dir))?;
    state.cleanup(game_dir);
    Ok(())
}

/// Rolls an interrupted transaction back. Ok(false) when there was none.
pub fn recover(game_dir: &Path) -> Result<bool> {
    let pending = pending_path(game_dir);
    if !pending.exists() {
        return Ok(false);
    }
    let state: JournalState = serde_json::from_str(&fs::read_to_string(&pending)?)?;
    let folder_ok = regex::Regex::new(r"^_DLSS5_Backup/\.transactions/[a-f0-9-]+$").unwrap().is_match(&state.folder);
    if state.version != 1 || !folder_ok {
        return Err(fail("errBackupInvalid", "pending-switch.json"));
    }
    // Validate everything before changing anything.
    for item in &state.files {
        safe_path(game_dir, &item.rel)?;
        if !item.snapshot.starts_with(&(state.folder.clone() + "/")) {
            return Err(fail("errBackupInvalid", &item.snapshot));
        }
        if item.existed && !safe_path(game_dir, &item.snapshot)?.exists() {
            return Err(fail("errBackendRecovery", &item.rel));
        }
    }
    for dir in &state.dirs {
        safe_path(game_dir, dir)?;
    }
    for item in state.files.iter().rev() {
        let target = safe_path(game_dir, &item.rel)?;
        if item.existed {
            if let Some(d) = target.parent() {
                fs::create_dir_all(d)?;
            }
            clear_readonly(&target);
            fs::copy(safe_path(game_dir, &item.snapshot)?, &target)?;
        } else if target.exists() {
            clear_readonly(&target);
            fs::remove_file(&target)?;
        }
    }
    let mut dirs = state.dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
    for dir in dirs {
        if let Ok(d) = safe_path(game_dir, &dir) {
            let _ = fs::remove_dir(d);
        }
    }
    fs::remove_file(&pending)?;
    state.cleanup(game_dir);
    Ok(true)
}

/// Windows carries a source's read-only attribute across a copy, and a
/// read-only DLL makes the next install or restore fail on overwrite.
pub fn clear_readonly(p: &Path) {
    if let Ok(meta) = fs::metadata(p) {
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(p, perms);
        }
    }
}
