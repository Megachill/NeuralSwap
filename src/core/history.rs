//! An append-only audit log of installs and restores (JSON lines), plus rows
//! imported from backups found in game folders — including backups written
//! by the original Electron DLSS 5 Swapper.

use super::apply::Manifest;
use super::scan::BACKUP_DIR;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub date: String,
    #[serde(default)]
    pub manifest_date: Option<String>,
    pub action: String,
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub exe: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub replaced: usize,
    #[serde(default)]
    pub added: usize,
    #[serde(default)]
    pub imported: bool,
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn fingerprint(r: &HistoryRow) -> String {
    format!("{}|{}|{}", r.dir.to_lowercase(), r.manifest_date.clone().unwrap_or_default(), r.action)
}

pub fn row(dir: &Path, m: Option<&Manifest>, action: &str, name: &str) -> HistoryRow {
    HistoryRow {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        dir: dir.to_string_lossy().to_string(),
        date: now_iso(),
        manifest_date: m.map(|m| m.date.clone()),
        action: action.to_string(),
        route: m.and_then(|m| m.route.clone()),
        exe: m.map(|m| m.game.exe.clone()),
        api: m.map(|m| m.game.api.clone()),
        replaced: m.map(|m| m.replaced.len()).unwrap_or(0),
        added: m.map(|m| m.added.len()).unwrap_or(0),
        imported: false,
    }
}

pub struct HistoryStore {
    pub file: PathBuf,
}

impl HistoryStore {
    pub fn saved(&self) -> Vec<HistoryRow> {
        let Ok(text) = fs::read_to_string(&self.file) else { return Vec::new() };
        let mut rows: Vec<HistoryRow> = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            // A truncated final write must not lose older operations.
            if let Ok(r) = serde_json::from_str::<HistoryRow>(line) {
                if let Some(pos) = rows.iter().position(|x| x.id == r.id) {
                    rows[pos] = r;
                } else {
                    rows.push(r);
                }
            }
        }
        rows
    }

    pub fn append(&self, rows: &[HistoryRow]) {
        if rows.is_empty() {
            return;
        }
        if let Some(d) = self.file.parent() {
            let _ = fs::create_dir_all(d);
        }
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&self.file) {
            let body: Vec<String> = rows.iter().filter_map(|r| serde_json::to_string(r).ok()).collect();
            let _ = write!(f, "\n{}\n", body.join("\n"));
            let _ = f.sync_all();
        }
    }

    pub fn record(&self, dir: &Path, m: Option<&Manifest>, action: &str, name: &str) {
        self.append(&[row(dir, m, action, name)]);
    }

    /// Saved rows plus backups discovered in `folders`, newest first.
    pub fn list(&self, folders: &[(PathBuf, String)]) -> Vec<HistoryRow> {
        let mut rows = self.saved();
        let mut seen: Vec<String> = rows.iter().map(fingerprint).collect();
        let mut imported = Vec::new();
        for (dir, name) in folders {
            let root = dir.join(BACKUP_DIR);
            if root.join("pending-switch.json").exists() {
                continue;
            }
            let Ok(entries) = fs::read_dir(&root) else { continue };
            for e in entries.flatten() {
                let fname = e.file_name().to_string_lossy().to_string();
                let done = fname.strip_prefix("manifest.json.done-").map(|s| s.to_string());
                if fname != "manifest.json" && done.is_none() {
                    continue;
                }
                let Ok(text) = fs::read_to_string(e.path()) else { continue };
                let Ok(m) = serde_json::from_str::<Manifest>(&text) else { continue };
                let mut r = row(dir, Some(&m), if done.is_some() { "restore" } else { "install" }, name);
                r.imported = true;
                r.date = done
                    .and_then(|ms| ms.parse::<i64>().ok())
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                    .unwrap_or_else(|| m.date.clone());
                r.id = format!("backup:{:x}", Sha256::digest(fingerprint(&r).as_bytes()));
                let fp = fingerprint(&r);
                if !seen.contains(&fp) {
                    seen.push(fp);
                    imported.push(r);
                }
            }
        }
        self.append(&imported);
        rows.extend(imported);
        rows.sort_by(|a, b| b.date.cmp(&a.date));
        rows
    }
}
