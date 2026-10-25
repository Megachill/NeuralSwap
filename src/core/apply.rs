//! The actual work: backs up, swaps the DLLs in place, drops the RenoDX
//! add-on beside the executable and deploys ReShade — inside a journaled
//! transaction, recording everything in `_DLSS5_Backup\manifest.json`.
//!
//! The manifest format is the one the original DLSS 5 Swapper writes, so a
//! game installed by either tool can be restored by the other.

use super::journal::{self, clear_readonly, rel_from, safe_path, JournalState};
use super::payload::{Payload, FEEDER_VERSION};
use super::scan::{self, inspect_reshade, norm_rel, BACKUP_DIR};
use super::{fail, guards, ini, pe, Event, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    /// The game already has DLSS: upgrade it and add the RenoDX consumer.
    Native,
    /// ShortFuse's RenoDX DLSS Tool (multipass), same install shape.
    Renodx,
    /// DLSS5-Feeder: a synthetic DLAA contract for games without DLSS.
    Feeder,
}

impl Route {
    pub fn key(self) -> &'static str {
        match self {
            Route::Native => "native",
            Route::Renodx => "renodx",
            Route::Feeder => "feeder",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Route::Native => "Native DLSS (RenoDX)",
            Route::Renodx => "RenoDX DLSS Tool (multipass)",
            Route::Feeder => "DLSS5-Feeder (No DLSS / Incompatible DLSS)",
        }
    }
    pub fn from_key(k: &str) -> Option<Route> {
        match k {
            "native" => Some(Route::Native),
            "renodx" => Some(Route::Renodx),
            "feeder" => Some(Route::Feeder),
            _ => None,
        }
    }
}

/// Which routes this build can install for an executable.
pub fn routes_for(bitness: u32, api: Option<&str>, api_label: Option<&str>) -> Vec<Route> {
    if bitness != 64 || api != Some("dxgi") || api_label == Some("DirectX 10") {
        return Vec::new();
    }
    let mut routes = Vec::new();
    if api_label == Some("DirectX 12") {
        routes.push(Route::Native);
    }
    routes.push(Route::Feeder);
    routes.push(Route::Renodx);
    routes
}

pub fn recommended_route(routes: &[Route], native_dlss: bool, previous: Option<&str>) -> Option<Route> {
    let wanted = match previous.and_then(Route::from_key) {
        Some(r) => r,
        None if native_dlss => Route::Native,
        None => Route::Feeder,
    };
    if routes.contains(&wanted) { Some(wanted) } else { routes.first().copied() }
}

// ---------------------------------------------------------------- manifest

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub dir: String,
    pub exe: String,
    pub api: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitness: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Replaced {
    pub rel: String,
    #[serde(default)]
    pub old_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReshadeInfo {
    #[serde(default)]
    pub installed_by_us: bool,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub files_added: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub backup_prefix: String,
    pub date: String,
    pub game: GameInfo,
    #[serde(default)]
    pub replaced: Vec<Replaced>,
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub added_dirs: Vec<String>,
    #[serde(default)]
    pub reshade: ReshadeInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Fields written by other versions (feeder, vulkanLayer, optiscaler…)
    /// survive a round trip untouched.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn backup_root(game_dir: &Path) -> PathBuf {
    game_dir.join(BACKUP_DIR)
}

fn manifest_path(game_dir: &Path) -> PathBuf {
    backup_root(game_dir).join("manifest.json")
}

pub fn read_manifest(game_dir: &Path) -> Result<Option<Manifest>> {
    let file = manifest_path(game_dir);
    if !file.exists() {
        return Ok(None);
    }
    let m: Manifest = serde_json::from_str(&fs::read_to_string(&file)?)?;
    if m.version != 1 {
        return Err(fail("errBackupInvalid", "manifest version"));
    }
    Ok(Some(m))
}

/// An active manifest describes the machine from before the first swap, not
/// the most recent click on Install. Reusing it is what lets a second install
/// (or a payload upgrade) still restore the genuine originals.
pub fn begin_manifest(game_dir: &Path, exe_path: &Path, api: &str) -> Result<Manifest> {
    let previous = read_manifest(game_dir)?;
    let mut m = previous.unwrap_or_else(|| Manifest {
        version: 1,
        backup_prefix: format!("originals/{}", uuid::Uuid::new_v4()),
        date: String::new(),
        game: GameInfo::default(),
        replaced: Vec::new(),
        added: Vec::new(),
        added_dirs: Vec::new(),
        reshade: ReshadeInfo::default(),
        route: None,
        extra: Map::new(),
    });
    m.version = 1;
    m.date = now_iso();
    let extra = std::mem::take(&mut m.game.extra);
    m.game = GameInfo {
        dir: game_dir.to_string_lossy().to_string(),
        exe: rel_from(game_dir, exe_path),
        api: api.to_string(),
        api_label: m.game.api_label.take(),
        bitness: m.game.bitness,
        extra,
    };
    Ok(m)
}

// ---------------------------------------------------------------- session

/// One install or restore against one game folder: the journal, the log
/// sink and the bookkeeping helpers that every write goes through.
pub struct Session<'l> {
    pub game_dir: PathBuf,
    journal: Option<JournalState>,
    log: &'l mut dyn FnMut(Event),
}

fn is_added(m: &Manifest, rel: &str) -> bool {
    let k = norm_rel(rel);
    m.added.iter().any(|a| norm_rel(a) == k)
}

fn remember_added(m: &mut Manifest, rel: &str) {
    if !is_added(m, rel) {
        m.added.push(rel.to_string());
    }
}

/// The first oldVersion is kept forever: it is the version in the backup.
fn remember_replacement(m: &mut Manifest, item: Replaced) {
    if is_added(m, &item.rel) {
        return;
    }
    let k = norm_rel(&item.rel);
    if let Some(prev) = m.replaced.iter_mut().find(|r| norm_rel(&r.rel) == k) {
        if item.new_version.is_some() {
            prev.new_version = item.new_version;
        }
        if prev.kind.is_none() {
            prev.kind = item.kind;
        }
        return;
    }
    m.replaced.push(item);
}

fn copy_over(src: &Path, dest: &Path) -> Result<()> {
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d)?;
    }
    clear_readonly(dest);
    fs::copy(src, dest).map_err(|e| fail("errIo", format!("{} → {}: {e}", src.display(), dest.display())))?;
    clear_readonly(dest);
    Ok(())
}

pub fn can_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".dlss5_write_test_{}", std::process::id()));
    match fs::write(&probe, b"x") {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn hook_for_api(api: &str) -> &'static str {
    match api {
        "opengl" => "opengl32.dll",
        "d3d9" => "d3d9.dll",
        _ => "dxgi.dll",
    }
}

fn is_addon_reshade(file: &Path) -> bool {
    let markers = pe::find_markers(file, &["ReShade", "Searching for add-ons"]);
    (pe::version_mentions(file, "ReShade") || markers.contains("ReShade")) && markers.contains("Searching for add-ons")
}

impl<'l> Session<'l> {
    pub fn new(game_dir: &Path, log: &'l mut dyn FnMut(Event)) -> Self {
        Session { game_dir: game_dir.to_path_buf(), journal: None, log }
    }

    pub fn log(&mut self, e: Event) {
        (self.log)(e)
    }

    fn rel(&self, target: &Path) -> String {
        rel_from(&self.game_dir, target)
    }

    fn original_path(&self, m: &Manifest, rel: &str) -> Result<PathBuf> {
        let prefix = &m.backup_prefix;
        if !prefix.is_empty() && !regex::Regex::new(r"^originals/[a-f0-9-]+$").unwrap().is_match(prefix) {
            return Err(fail("errBackupInvalid", "backupPrefix"));
        }
        let rel = if prefix.is_empty() { rel.to_string() } else { format!("{}\\{}", prefix.replace('/', "\\"), rel) };
        safe_path(&backup_root(&self.game_dir), &rel)
    }

    fn capture(&mut self, target: &Path) -> Result<()> {
        if let Some(j) = self.journal.as_mut() {
            j.capture(&self.game_dir, target)?;
        }
        Ok(())
    }

    pub fn save_manifest(&mut self, m: &Manifest) -> Result<()> {
        let file = manifest_path(&self.game_dir);
        self.capture(&file)?;
        journal::atomic_json(&file, m)
    }

    fn remember_missing_parents(&self, m: &mut Manifest, target: &Path) {
        let root = self.game_dir.to_string_lossy().to_lowercase();
        let mut missing = Vec::new();
        let mut current = target.parent().map(Path::to_path_buf);
        while let Some(c) = current {
            if c.to_string_lossy().to_lowercase().trim_end_matches('\\') == root.trim_end_matches('\\') {
                break;
            }
            if !c.to_string_lossy().to_lowercase().starts_with(&root) {
                break;
            }
            if !c.exists() {
                missing.push(self.rel(&c));
            }
            current = c.parent().map(Path::to_path_buf);
        }
        for rel in missing.into_iter().rev() {
            let k = norm_rel(&rel);
            if !m.added_dirs.iter().any(|d| norm_rel(d) == k) {
                m.added_dirs.push(rel);
            }
        }
    }

    /// Backs up (or records as new) a file before anything writes to it.
    pub fn track_before_write(&mut self, m: &mut Manifest, target: &Path, old_version: Option<Option<String>>, new_version: Option<String>, kind: Option<&str>) -> Result<String> {
        let rel = self.rel(target);
        safe_path(&self.game_dir, &rel)?;
        if rel.split(['\\', '/']).next().map(|s| s.eq_ignore_ascii_case(BACKUP_DIR)).unwrap_or(false) {
            return Err(fail("errUnsafeTarget", &rel));
        }
        self.capture(target)?;
        self.remember_missing_parents(m, target);
        if target.exists() {
            if !is_added(m, &rel) {
                let backup = self.original_path(m, &rel)?;
                if !backup.exists() {
                    copy_over(target, &backup)?;
                }
                remember_replacement(m, Replaced {
                    rel: rel.clone(),
                    old_version: old_version.unwrap_or_else(|| pe::file_version(target)),
                    new_version,
                    kind: kind.map(String::from),
                });
            }
        } else {
            remember_added(m, &rel);
        }
        Ok(rel)
    }

    pub fn copy_tracked(&mut self, m: &mut Manifest, src: &Path, dest: &Path, old_version: Option<Option<String>>, kind: Option<&str>) -> Result<String> {
        let new_version = pe::file_version(src);
        let rel = self.track_before_write(m, dest, old_version, new_version, kind)?;
        self.save_manifest(m)?;
        copy_over(src, dest)?;
        Ok(rel)
    }

    pub fn write_tracked(&mut self, m: &mut Manifest, dest: &Path, text: &str, kind: Option<&str>) -> Result<String> {
        let rel = self.track_before_write(m, dest, None, None, kind)?;
        self.save_manifest(m)?;
        if let Some(d) = dest.parent() {
            fs::create_dir_all(d)?;
        }
        clear_readonly(dest);
        fs::write(dest, text)?;
        Ok(rel)
    }

    fn copy_tree_tracked(&mut self, m: &mut Manifest, src_root: &Path, dest_root: &Path) -> Result<usize> {
        let mut count = 0;
        let mut queue = vec![PathBuf::new()];
        while let Some(rel_dir) = queue.pop() {
            let Ok(entries) = fs::read_dir(src_root.join(&rel_dir)) else { continue };
            for e in entries.flatten() {
                let rel = rel_dir.join(e.file_name());
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    queue.push(rel);
                } else if ft.is_file() {
                    self.copy_tracked(m, &src_root.join(&rel), &dest_root.join(&rel), None, Some("shader"))?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Keeps ReShade from starting with our add-on switched off.
    fn enable_addon_in_ini(&mut self, m: &mut Manifest, exe_dir: &Path, addon: &str) -> Result<()> {
        let path = exe_dir.join("ReShade.ini");
        if !path.exists() {
            return Ok(());
        }
        let text = ini::read_text(&path);
        let lower = addon.to_lowercase();
        let stem = lower.trim_end_matches(".addon64").trim_end_matches(".addon32").trim_end_matches(".addon").to_string();
        let re = regex::Regex::new(r"(?m)^DisabledAddons=(.*)$").unwrap();
        if let Some(c) = re.captures(&text) {
            if c[1].to_lowercase().contains(&stem) {
                let kept: Vec<&str> = c[1].split(',').filter(|n| !n.to_lowercase().contains(&stem)).collect();
                let new = re.replace(&text, format!("DisabledAddons={}", kept.join(",")).as_str()).to_string();
                self.write_tracked(m, &path, &new, Some("config"))?;
                self.log(Event::info(format!("Re-enabled {addon} in ReShade.ini")));
            }
        }
        Ok(())
    }

    /// Two RenoDX neural consumers in one folder fight over one registration;
    /// every other renodx-dlss* add-on beside ours is moved into the backup.
    fn set_aside_rival_consumers(&mut self, m: &mut Manifest, dir: &Path, keep: &str) -> Result<()> {
        let re = regex::Regex::new(r"(?i)^renodx-dlss.*\.addon64$").unwrap();
        let Ok(entries) = fs::read_dir(dir) else { return Ok(()) };
        let names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        for name in names {
            if !re.is_match(&name) || name.eq_ignore_ascii_case(keep) {
                continue;
            }
            let file = dir.join(&name);
            let ours = is_added(m, &self.rel(&file));
            let rel = self.track_before_write(m, &file, None, None, Some("addon"))?;
            self.save_manifest(m)?;
            clear_readonly(&file);
            fs::remove_file(&file)?;
            self.log(Event::info(if ours { format!("Removed {rel}") } else { format!("Set aside rival consumer {name} (restorable)") }));
        }
        Ok(())
    }

    /// A game-local D3DCompiler_47.dll from before Windows 10 cannot compile
    /// the neural pass; retire it into the backup when System32 has a newer one.
    fn retire_old_shader_compiler(&mut self, m: &mut Manifest, exe_dir: &Path) -> Result<()> {
        let Some((file, version)) = scan::old_shader_compiler(exe_dir) else { return Ok(()) };
        let system = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32");
        if !system.join("D3DCompiler_47.dll").exists() || scan::old_shader_compiler(&system).is_some() {
            return Ok(());
        }
        let rel = self.track_before_write(m, &file, Some(Some(version.clone())), None, Some("shaderCompiler"))?;
        self.save_manifest(m)?;
        clear_readonly(&file);
        fs::remove_file(&file)?;
        self.log(Event::warn(format!("Retired outdated {rel} ({version}) — Windows' own copy will be used")));
        Ok(())
    }

    fn install_reshade(&mut self, m: &mut Manifest, payload: &Payload, exe_path: &Path, api: &str, bitness: u32) -> Result<()> {
        let exe_dir = exe_path.parent().unwrap();
        let hook = hook_for_api(api);
        let hook_path = exe_dir.join(hook);
        let bundled = if bitness == 32 { payload.reshade32.clone() } else { payload.reshade64.clone() };
        let Some(bundled) = bundled else { return Err(fail("errReShadeSetupMissing", "")) };
        if pe::bitness(&bundled) != Some(bitness) || !is_addon_reshade(&bundled) {
            return Err(fail("errReShadeArchitecture", bundled.display().to_string()));
        }
        let existed = hook_path.exists();
        self.copy_tracked(m, &bundled, &hook_path, None, Some("reshade"))?;
        m.reshade.installed_by_us = !existed || is_added(m, &self.rel(&hook_path));
        m.reshade.file = Some(hook.to_string());
        self.save_manifest(m)?;
        self.log(Event::ok(format!(
            "ReShade {} (add-on build) installed as {hook}",
            pe::file_version(&hook_path).unwrap_or_else(|| "?".into())
        )));
        Ok(())
    }
}

// ---------------------------------------------------------------- install

pub struct InstallConfig {
    pub game_dir: PathBuf,
    pub exe_path: PathBuf,
    pub api: String,
    pub api_label: Option<String>,
    pub bitness: u32,
    pub route: Route,
    pub anti_cheat_acknowledged: bool,
    pub payload: Payload,
    /// Extra user-chosen .addon64 files installed beside the RenoDX consumer.
    pub companions: Vec<PathBuf>,
}

/// Refuses to stack a second ReShade or overwrite another graphics injector.
fn assert_loader_compatible(cfg: &InstallConfig, manifest: Option<&Manifest>) -> Result<()> {
    let dir = cfg.exe_path.parent().unwrap();
    let mut owned: Vec<String> = Vec::new();
    if let Some(m) = manifest {
        owned.extend(m.added.iter().map(|a| norm_rel(a)));
        owned.extend(m.replaced.iter().map(|r| norm_rel(&r.rel)));
    }
    let reshade = inspect_reshade(dir);
    if reshade.installed && reshade.kind.as_deref() == Some("asi") && (cfg.route == Route::Feeder || !reshade.addon_support) {
        return Err(fail("errLoaderConflict", reshade.file.unwrap_or_default()));
    }
    let hook_re = regex::Regex::new(r"(?i)^(dxgi|ddraw|d3d8|d3d9|d3d10|d3d10core|d3d11|d3d12|opengl32)\.dll$").unwrap();
    let hook = hook_for_api(&cfg.api);
    let mut reshade_hooks = 0;
    for e in fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !hook_re.is_match(&name) {
            continue;
        }
        let file = e.path();
        let rel = rel_from(&cfg.game_dir, &file);
        safe_path(&cfg.game_dir, &rel)?;
        let is_reshade = pe::version_mentions(&file, "ReShade");
        if is_reshade {
            reshade_hooks += 1;
            if reshade_hooks > 1 {
                return Err(fail("errLoaderConflict", format!("multiple ReShade hooks in {}", dir.display())));
            }
        }
        if owned.contains(&norm_rel(&rel)) {
            continue;
        }
        if is_reshade
            && pe::bitness(&file) == Some(cfg.bitness)
            && reshade.kind.as_deref() != Some("asi")
            && (if cfg.route == Route::Feeder { name.eq_ignore_ascii_case(hook) } else { reshade.addon_support })
        {
            continue;
        }
        return Err(fail("errLoaderConflict", rel));
    }
    Ok(())
}

fn config_paths(game_dir: &Path, exe_path: &Path) -> Vec<PathBuf> {
    let dir = exe_path.parent().unwrap();
    let reshade = dir.join("ReShade.ini");
    let preset = ini::preset_path(dir, &ini::read_text(&reshade));
    let mut files = vec![reshade, dir.join("dlss5-feed.cfg"), dir.join("host64").join("ReShade.ini")];
    if preset.to_string_lossy().to_lowercase().starts_with(&game_dir.to_string_lossy().to_lowercase()) {
        files.push(preset);
    }
    files
}

fn apply_native(s: &mut Session, cfg: &InstallConfig) -> Result<Manifest> {
    let exe_dir = cfg.exe_path.parent().unwrap().to_path_buf();
    let payload = &cfg.payload;
    if !payload.has_neural_rendering() {
        return Err(fail("errNoNeuralRuntime", ""));
    }
    let multipass = cfg.route == Route::Renodx;
    if multipass && payload.multipass_addon.is_none() {
        return Err(fail("errMultipassMissing", ""));
    }

    let scan = scan::scan_game(&cfg.game_dir);
    let mut m = begin_manifest(&cfg.game_dir, &cfg.exe_path, &cfg.api)?;
    m.route = Some(cfg.route.key().into());
    m.game.api_label = cfg.api_label.clone();
    s.retire_old_shader_compiler(&mut m, &exe_dir)?;

    // 1) Upgrade the game's own DLSS SR / NR runtimes in place — including
    //    Unreal's nested ThirdParty folders. Streamline, frame generation
    //    and ray reconstruction belong to the game's SDK integration and are
    //    left alone.
    let runtime = regex::Regex::new(r"(?i)^nvngx_dlss(?:nr)?\.dll$").unwrap();
    let existing: Vec<_> = scan.dlss_files.iter().filter(|f| runtime.is_match(&f.name)).cloned().collect();
    for file in &existing {
        let Some(rep) = payload.file(&file.name) else { continue };
        if file.bitness != Some(cfg.bitness) || pe::bitness(&rep.path) != Some(cfg.bitness) {
            s.log(Event::warn(format!("Skipped {} (different architecture)", file.rel)));
            continue;
        }
        if rep.version.is_some() && rep.version == file.version {
            s.log(Event::info(format!("{} is already {}", file.rel, file.version.clone().unwrap_or_default())));
            continue;
        }
        s.copy_tracked(&mut m, &rep.path, &file.path, Some(file.version.clone()), None)?;
        s.log(Event::ok(format!(
            "Replaced {}  {} → {}",
            file.rel,
            file.version.clone().unwrap_or_else(|| "?".into()),
            rep.version.clone().unwrap_or_else(|| "?".into())
        )));
    }

    // 2) The neural-rendering runtime the add-on loads sits beside the exe,
    //    plus base DLSS when the game has none of its own at this bitness.
    let mut beside = vec!["nvngx_dlssnr.dll"];
    if !existing.iter().any(|f| f.name.eq_ignore_ascii_case("nvngx_dlss.dll") && f.bitness == Some(cfg.bitness)) {
        beside.push("nvngx_dlss.dll");
    }
    for name in beside {
        let Some(item) = payload.file(name) else { continue };
        if pe::bitness(&item.path) != Some(cfg.bitness) {
            return Err(fail("errReShadeArchitecture", item.path.display().to_string()));
        }
        let dest = exe_dir.join(name);
        let rel = rel_from(&cfg.game_dir, &dest);
        if existing.iter().any(|f| norm_rel(&f.rel) == norm_rel(&rel)) {
            continue; // already handled by the in-place upgrade above
        }
        if dest.exists() {
            if pe::bitness(&dest) != Some(cfg.bitness) {
                return Err(fail("errRuntimeArchitecture", rel));
            }
            let current = pe::file_version(&dest);
            if current == item.version {
                s.log(Event::info(format!("{rel} is already {}", current.unwrap_or_default())));
                continue;
            }
            s.copy_tracked(&mut m, &item.path, &dest, Some(current.clone()), None)?;
            s.log(Event::ok(format!("Replaced {rel}  {} → {}", current.unwrap_or_else(|| "?".into()), item.version.clone().unwrap_or_default())));
        } else {
            s.copy_tracked(&mut m, &item.path, &dest, None, None)?;
            s.log(Event::ok(format!("Added {rel}  {}", item.version.clone().unwrap_or_default())));
        }
    }

    // 3) The RenoDX add-on — and no other neural consumer beside it.
    let addon_src = if multipass { payload.multipass_addon.clone() } else { payload.addon.clone() };
    let addon_name = addon_src.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
    if let (Some(src), Some(name)) = (&addon_src, &addon_name) {
        s.set_aside_rival_consumers(&mut m, &exe_dir, name)?;
        s.copy_tracked(&mut m, src, &exe_dir.join(name), None, Some("addon"))?;
        s.log(Event::ok(format!("Add-on installed: {name}")));
    }

    // 4) ReShade — the add-on is loaded by ReShade, so without it nothing runs.
    let before = inspect_reshade(&exe_dir);
    let installing_fresh = !before.installed || (before.kind.as_deref() == Some("proxy") && !before.addon_support);
    m.reshade.file = before.file.clone();
    if installing_fresh {
        s.install_reshade(&mut m, payload, &cfg.exe_path, &cfg.api, cfg.bitness)?;
    } else {
        s.log(Event::info(format!(
            "ReShade {} already present as {} ({}{})",
            before.version.clone().unwrap_or_default(),
            before.file.clone().unwrap_or_default(),
            before.kind.clone().unwrap_or_default(),
            if before.addon_support { ", add-on support" } else { "" }
        )));
        if pe::compare_versions(payload.reshade_version.as_deref(), before.version.as_deref()) > 0 {
            s.log(Event::info(format!("A newer ReShade ({}) is in the payload", payload.reshade_version.clone().unwrap_or_default())));
        }
    }

    if let Some(name) = &addon_name {
        s.enable_addon_in_ini(&mut m, &exe_dir, name)?;
    }
    s.save_manifest(&m)?;
    Ok(m)
}

fn apply_feeder(s: &mut Session, cfg: &InstallConfig) -> Result<Manifest> {
    let exe_dir = cfg.exe_path.parent().unwrap().to_path_buf();
    let payload = &cfg.payload;
    if !payload.has_neural_rendering() {
        return Err(fail("errNoNeuralRuntime", ""));
    }
    if cfg.bitness != 64 || cfg.api != "dxgi" {
        return Err(fail("errFeederApiUnsupported", format!("{} / {}-bit", cfg.api, cfg.bitness)));
    }
    if !payload.feeder.ok64 {
        return Err(fail("errFeederSupportMissing", payload.feeder.dir.display().to_string()));
    }
    let (Some(neural), Some(dlss)) = (payload.file("nvngx_dlssnr.dll").cloned(), payload.file("nvngx_dlss.dll").cloned()) else {
        return Err(fail("errNoNeuralRuntime", ""));
    };

    let mut m = begin_manifest(&cfg.game_dir, &cfg.exe_path, &cfg.api)?;
    m.route = Some("feeder".into());
    m.game.bitness = Some(cfg.bitness);
    m.game.api_label = cfg.api_label.clone();
    s.retire_old_shader_compiler(&mut m, &exe_dir)?;

    if !inspect_reshade(&exe_dir).addon_support {
        s.install_reshade(&mut m, payload, &cfg.exe_path, "dxgi", 64)?;
    }
    s.copy_tracked(&mut m, &payload.feeder.addon64, &exe_dir.join("dlss5-feed.addon64"), None, Some("feeder"))?;
    s.log(Event::ok("Add-on installed: dlss5-feed.addon64"));
    let n = s.copy_tree_tracked(&mut m, &payload.feeder.shader_root, &exe_dir.join("reshade-shaders"))?;
    s.log(Event::ok(format!("Deployed {n} Feeder / VORT shader files")));
    // VORT motion vectors (provider 2) ship with the payload.
    let provider = 2;
    m.extra.insert("feeder".into(), serde_json::json!({ "version": FEEDER_VERSION, "provider": provider }));
    let verifier = payload.feeder.dir.join("Verify-DLSS5Feeder.ps1");
    if verifier.exists() {
        s.copy_tracked(&mut m, &verifier, &exe_dir.join("Verify-DLSS5Feeder.ps1"), None, Some("diagnostics"))?;
    }
    let shaders = exe_dir.join("reshade-shaders");
    for rel in ["Shaders\\DLSS5_Feed.fx", "Shaders\\vort_Motion.fx", "Shaders\\ReShade.fxh", "Shaders\\ReShadeUI.fxh"] {
        if !shaders.join(rel).exists() {
            return Err(fail("errShaderInstall", rel));
        }
    }

    let game_ini_path = exe_dir.join("ReShade.ini");
    let mut game_ini = ini::configure_consumer(&ini::configure_game_reshade(&ini::read_text(&game_ini_path), provider));
    let mut preset = ini::preset_path(&exe_dir, &game_ini);
    if !preset.to_string_lossy().to_lowercase().starts_with(&cfg.game_dir.to_string_lossy().to_lowercase()) {
        // Never modify a shared preset outside the game.
        game_ini = ini::set(&game_ini, "GENERAL", "PresetPath", ".\\ReShadePreset.ini");
        preset = exe_dir.join("ReShadePreset.ini");
    }
    s.write_tracked(&mut m, &game_ini_path, &game_ini, Some("config"))?;
    let preset_text = ini::configure_preset(&ini::read_text(&preset), provider);
    s.write_tracked(&mut m, &preset, &preset_text, Some("config"))?;
    let cfg_path = exe_dir.join("dlss5-feed.cfg");
    let feed = ini::configure_feed(&ini::read_text(&cfg_path));
    s.write_tracked(&mut m, &cfg_path, &feed, Some("config"))?;
    s.log(Event::ok("Configured ReShade.ini, preset and dlss5-feed.cfg"));

    let consumer = "renodx-dlss5.addon64";
    s.set_aside_rival_consumers(&mut m, &exe_dir, consumer)?;
    for (src, name, kind) in [
        (payload.feeder.host_addon.clone(), consumer.to_string(), "addon"),
        (neural.path.clone(), neural.name.clone(), "runtime"),
        (dlss.path.clone(), dlss.name.clone(), "runtime"),
    ] {
        let rel = s.copy_tracked(&mut m, &src, &exe_dir.join(&name), None, Some(kind))?;
        s.log(Event::ok(format!("Added {rel}  {}", pe::file_version(&src).unwrap_or_default())));
    }
    s.enable_addon_in_ini(&mut m, &exe_dir, "dlss5-feed.addon64")?;
    s.enable_addon_in_ini(&mut m, &exe_dir, consumer)?;
    s.save_manifest(&m)?;
    Ok(m)
}

/// Install (or reinstall / switch route) inside a journaled transaction.
pub fn install(cfg: &InstallConfig, log: &mut dyn FnMut(Event)) -> Result<Manifest> {
    let game_dir = &cfg.game_dir;
    if let Some(root) = scan::managed_mod_root(game_dir, Some(&cfg.exe_path)) {
        return Err(fail("errManagedModpack", root.display().to_string()));
    }
    if !can_write(game_dir) || !can_write(cfg.exe_path.parent().unwrap()) {
        return Err(fail("errNoWriteAccess", game_dir.display().to_string()));
    }
    if scan::anti_cheat_present(game_dir) && !cfg.anti_cheat_acknowledged {
        return Err(fail("errAntiCheatConsent", ""));
    }
    if !routes_for(cfg.bitness, Some(&cfg.api), cfg.api_label.as_deref()).contains(&cfg.route) {
        return Err(fail("errRouteUnsupported", format!("{} on {} {}-bit", cfg.route.label(), cfg.api_label.clone().unwrap_or(cfg.api.clone()), cfg.bitness)));
    }
    guards::assert_game_closed(game_dir, Some(&cfg.exe_path))?;

    let old = read_manifest(game_dir)?;
    let changed = old.as_ref().map(|o| {
        o.route.as_deref().unwrap_or("native") != cfg.route.key()
            || o.game.api != cfg.api
            || !o.game.exe.eq_ignore_ascii_case(&rel_from(game_dir, &cfg.exe_path))
    }).unwrap_or(false);

    let mut s = Session::new(game_dir, log);
    s.journal = Some(journal::begin(game_dir)?);
    let result = (|| -> Result<Manifest> {
        if old.is_none() {
            // Capture an untouched custom preset's original bytes before the
            // first managed session.
            let mut initial = begin_manifest(game_dir, &cfg.exe_path, &cfg.api)?;
            for file in config_paths(game_dir, &cfg.exe_path) {
                if file.exists() {
                    s.track_before_write(&mut initial, &file, None, None, Some("config"))?;
                }
            }
            s.save_manifest(&initial)?;
        }
        if changed {
            let o = old.as_ref().unwrap();
            s.log(Event::info(format!("Switching route {} → {}", o.route.as_deref().unwrap_or("native"), cfg.route.key())));
            restore_files(&mut s, o)?;
        }
        assert_loader_compatible(cfg, if changed { None } else { old.as_ref() })?;
        let mut m = match cfg.route {
            Route::Native | Route::Renodx => apply_native(&mut s, cfg)?,
            Route::Feeder => apply_feeder(&mut s, cfg)?,
        };
        let exe_dir = cfg.exe_path.parent().unwrap().to_path_buf();
        for companion in &cfg.companions {
            let Some(name) = companion.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
            s.copy_tracked(&mut m, companion, &exe_dir.join(&name), None, Some("addon"))?;
            s.enable_addon_in_ini(&mut m, &exe_dir, &name)?;
            s.log(Event::ok(format!("Companion add-on installed: {name}")));
        }
        s.save_manifest(&m)?;
        Ok(m)
    })();

    match result {
        Ok(m) => {
            let j = s.journal.take().unwrap();
            journal::commit(game_dir, &j)?;
            s.log(Event::ok(format!("Done — {} replaced, {} added. Launch the game and enable DLSS in its settings.", m.replaced.len(), m.added.len())));
            Ok(m)
        }
        Err(e) => {
            s.journal = None;
            s.log(Event { level: super::Level::Error, text: e.to_string() });
            match journal::recover(game_dir) {
                Ok(_) => {
                    s.log(Event::warn("Rolled back: every file is exactly as it was before this install."));
                    Err(e)
                }
                Err(re) => Err(fail("errBackendRecovery", format!("{e}; rollback failed: {re}"))),
            }
        }
    }
}

// ---------------------------------------------------------------- restore

fn restore_files(s: &mut Session, m: &Manifest) -> Result<()> {
    let game_dir = s.game_dir.clone();
    if m.version != 1 {
        return Err(fail("errBackupInvalid", ""));
    }
    // Validate every path and backup before restoring the first file.
    for item in &m.replaced {
        safe_path(&game_dir, &item.rel)?;
        if !s.original_path(m, &item.rel)?.exists() {
            return Err(fail("errBackupInvalid", format!("missing original backup: {}", item.rel)));
        }
    }
    for rel in m.added.iter().chain(m.added_dirs.iter()) {
        safe_path(&game_dir, rel)?;
    }
    safe_path(&game_dir, &m.game.exe)?;

    for item in &m.replaced {
        let backup = s.original_path(m, &item.rel)?;
        let target = safe_path(&game_dir, &item.rel)?;
        s.capture(&target)?;
        copy_over(&backup, &target)?;
        s.log(Event::ok(format!(
            "Restored {}{}",
            item.rel,
            item.old_version.as_ref().map(|v| format!("  ({v})")).unwrap_or_default()
        )));
    }
    for rel in &m.added {
        let target = safe_path(&game_dir, rel)?;
        if target.exists() {
            s.capture(&target)?;
            clear_readonly(&target);
            fs::remove_file(&target)?;
            s.log(Event::info(format!("Deleted {rel}")));
        }
    }
    let mut dirs = m.added_dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
    for rel in dirs {
        let target = safe_path(&game_dir, &rel)?;
        // Only empty folders: anything the user or game added survives.
        if target.exists() && fs::remove_dir(&target).is_ok() {
            s.log(Event::info(format!("Deleted {rel}\\")));
        }
    }
    let exe_dir = safe_path(&game_dir, &m.game.exe)?.parent().unwrap().to_path_buf();
    let mut leftovers = m.reshade.files_added.clone();
    if m.reshade.installed_by_us {
        if let Some(f) = &m.reshade.file {
            leftovers.push(f.clone());
        }
    }
    for name in leftovers {
        let rel = rel_from(&game_dir, &exe_dir.join(&name));
        let Ok(target) = safe_path(&game_dir, &rel) else { continue };
        if m.replaced.iter().any(|r| norm_rel(&r.rel) == norm_rel(&rel)) || !target.exists() {
            continue;
        }
        if target.is_dir() {
            if fs::remove_dir(&target).is_ok() {
                s.log(Event::info(format!("Deleted {name}")));
            }
        } else {
            s.capture(&target)?;
            clear_readonly(&target);
            if fs::remove_file(&target).is_ok() {
                s.log(Event::info(format!("Deleted {name}")));
            }
        }
    }
    Ok(())
}

/// Puts every original back, deletes what was added, and retires the
/// manifest as `manifest.json.done-<ms>` (kept for history).
pub fn restore(game_dir: &Path, log: &mut dyn FnMut(Event)) -> Result<Option<Manifest>> {
    let old = match read_manifest(game_dir) {
        Ok(m) => m,
        Err(e) if !journal::pending_path(game_dir).exists() => return Err(e),
        Err(_) => None,
    };
    let exe = old.as_ref().map(|m| safe_path(game_dir, &m.game.exe)).transpose()?;
    guards::assert_game_closed(game_dir, exe.as_deref())?;
    let mut s = Session::new(game_dir, log);
    if journal::recover(game_dir)? {
        s.log(Event::warn("Recovered an interrupted install first."));
    }
    let Some(m) = read_manifest(game_dir)? else {
        return if old.is_none() { Err(fail("errNoBackup", "")) } else { Ok(None) };
    };
    restore_files(&mut s, &m)?;
    let file = manifest_path(game_dir);
    let stamp = chrono::Utc::now().timestamp_millis();
    fs::rename(&file, PathBuf::from(format!("{}.done-{stamp}", file.display())))?;
    s.log(Event::ok(format!("Restore complete — {} files put back, {} removed.", m.replaced.len(), m.added.len())));
    Ok(Some(m))
}
