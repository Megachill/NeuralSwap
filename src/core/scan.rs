//! Works out everything an install needs to know about a game folder: which
//! executable is the game, which rendering API it uses, where the existing
//! DLSS/Streamline files live, and whether ReShade is already there.

use super::pe;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

pub const BACKUP_DIR: &str = "_DLSS5_Backup";
const MAX_SCAN_DEPTH: usize = 12;
const UNDETECTED_LIMIT: usize = 8;

const SKIP_DIRS: &[&str] = &[
    "_dlss5_backup", "reshade-shaders", "host64", "node_modules", ".git",
    // Asset trees hold tens of thousands of files but never the runtime DLL
    // or executable we need.
    "paks", "movies", "screenshots", "saved", "logs", "mods", "downloads", "overwrite", "profiles",
    "_redist", "prerequisites", "directx", "redist", "redistributable", "redistributables",
    "_commonredist", "dotnet",
    // Shipped installers and vendor helpers keep their own executables around.
    "installer_resources", "installer", "installers", "support", "vcredist", "_support",
    "directx_redist", "eaanticheat", "easyanticheat", "battleye",
    // Never touch copies the user (or another tool) parked as a backup.
    "backup", "backups", "_backup", "bak", "old", "original", "originals",
];

// Installers, launchers and anti-cheat helpers are never the game itself.
static NOT_A_GAME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(unins|setup|install|vcredist|vc_redist|dxsetup|dxwebsetup|oalinst|uninstall|crashreport|crashhandler|easyanticheat|eac|battleye|be_service|launcher|activation|patch|update|dotnetfx|touchup|rapidcrc|autorun|autoplay|quicksfv|readme|config|benchmark|report|helper|service|cleanup|modorganizer|redlauncher|skse\d*_loader|hlds\b|srcds\b|steamerrorreporter|dgvoodoocpl|reshade_setup)").unwrap()
});
pub static DLSS_FILE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(nvngx_dlss[a-z_]*\.dll|nvngx\.dll|_nvngx\.dll)$").unwrap());
pub static STREAMLINE_FILE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^sl\.[a-z_]+\.dll$").unwrap());
const RESHADE_HOOKS: &[&str] = &["dxgi.dll", "d3d12.dll", "d3d11.dll", "d3d9.dll", "opengl32.dll", "dinput8.dll"];

// Libraries that mention Direct3D without being a game's renderer.
static NOT_A_RENDERER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:libglesv2|libegl|libcef|chrome_elf|cef|d3dcompiler_\d+|dxcompiler|dxil|vulkan-1|opengl32|d3d\d+|d3d\d+core|dxgi|ddraw|d3dimm|nvngx.*|sl\..*|amd_.*|ffx_.*|reshade\d*|dlss5.*|dgvoodoo.*|qt\d.*|mono.*|steam_api.*|galaxy.*|eossdk.*|discord.*|gameoverlayrenderer.*|renodx.*)\.dll$").unwrap()
});
static NOT_OFFERED: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^rendersystem").unwrap());

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApiChoice {
    pub api: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExeCandidate {
    pub path: PathBuf,
    pub rel: String,
    pub name: String,
    pub size: u64,
    pub depth: usize,
    pub api: Option<String>,
    pub api_label: Option<String>,
    pub via: String,
    pub bitness: u32,
    pub dx12: bool,
    #[serde(default)]
    pub declared: bool,
    #[serde(default)]
    pub encrypted: bool,
    pub api_choices: Vec<ApiChoice>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DllFile {
    pub path: PathBuf,
    pub rel: String,
    pub name: String,
    pub version: Option<String>,
    pub bitness: Option<u32>,
    pub depth: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReshadeState {
    pub installed: bool,
    pub file: Option<String>,
    pub kind: Option<String>,
    pub version: Option<String>,
    pub addon_support: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstallState {
    pub route: String,
    pub api: Option<String>,
    pub exe: Option<String>,
    pub added: Vec<String>,
    pub date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameScan {
    pub game_dir: PathBuf,
    pub game_name: String,
    pub exe_candidates: Vec<ExeCandidate>,
    pub dlss_files: Vec<DllFile>,
    pub streamline_files: Vec<DllFile>,
    pub addon_present: Option<String>,
    pub empty_reason: Option<String>,
    pub reshade: ReshadeState,
    pub install: Option<InstallState>,
    pub has_backup: bool,
    pub pending_recovery: bool,
    pub anti_cheat: bool,
}

impl GameScan {
    pub fn chosen(&self) -> Option<&ExeCandidate> {
        self.exe_candidates.first()
    }

    pub fn candidate(&self, rel: Option<&str>) -> Option<&ExeCandidate> {
        match rel {
            Some(r) => self
                .exe_candidates
                .iter()
                .find(|c| c.rel.eq_ignore_ascii_case(r))
                .or_else(|| self.chosen()),
            None => self.chosen(),
        }
    }

    /// The base DLSS runtime the game ships, nearest to the executable.
    pub fn primary_dlss(&self, target: Option<&ExeCandidate>) -> Option<&DllFile> {
        select_primary_dlss(&self.dlss_files, target)
    }

    /// A copied-in nvngx DLL alone does not prove that the game has native
    /// NGX calls: one this app added itself does not count.
    pub fn native_dlss_present(&self, target: Option<&ExeCandidate>) -> bool {
        let Some(file) = self.primary_dlss(target) else { return false };
        let rel = norm_rel(&file.rel);
        !self
            .install
            .as_ref()
            .map(|i| i.added.iter().any(|a| norm_rel(a) == rel))
            .unwrap_or(false)
    }
}

pub fn norm_rel(rel: &str) -> String {
    rel.replace('/', "\\").to_lowercase()
}

pub fn rel_of(root: &Path, full: &Path) -> String {
    full.strip_prefix(root).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| full.to_string_lossy().to_string())
}

fn walk(root: &Path, include_content: bool, mut on_file: impl FnMut(&Path, &str, usize)) {
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            let full = entry.path();
            if ft.is_dir() {
                let lower = name.to_lowercase();
                // Unreal's Content tree is assets only and enormous, but
                // modern Xbox installs put the whole game under <Game>\Content.
                let skip_content = lower == "content" && !include_content;
                if depth < MAX_SCAN_DEPTH && !skip_content && !SKIP_DIRS.contains(&lower.as_str()) {
                    queue.push_back((full, depth + 1));
                }
            } else if ft.is_file() {
                on_file(&full, &name, depth);
            }
        }
    }
}

pub fn find_case_insensitive(dir: &Path, wanted: &str) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(wanted))
        .map(|e| e.path())
}

/// The version resource of an Addon build looks identical to a plain one, so
/// the only honest test is whether the binary carries the add-on loader.
pub fn has_addon_support(file: &Path) -> bool {
    pe::file_contains(file, "Searching for add-ons")
}

/// ReShade proxies itself as a system DLL, so a dxgi.dll next to the game is
/// either ReShade or something else entirely — the version resource tells
/// them apart. Modded games instead run it as ReShade.asi.
pub fn inspect_reshade(exe_dir: &Path) -> ReshadeState {
    let mut names: Vec<String> = RESHADE_HOOKS.iter().map(|s| s.to_string()).collect();
    if let Ok(entries) = fs::read_dir(exe_dir) {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.to_lowercase().ends_with(".asi") {
                names.push(n);
            }
        }
    }
    for name in names {
        let file = exe_dir.join(&name);
        if !file.exists() {
            continue;
        }
        if pe::version_mentions(&file, "ReShade") {
            return ReshadeState {
                installed: true,
                kind: Some(if name.to_lowercase().ends_with(".asi") { "asi" } else { "proxy" }.into()),
                version: pe::file_version(&file),
                addon_support: has_addon_support(&file),
                file: Some(name),
            };
        }
    }
    ReshadeState::default()
}

const API_MARKERS: &[&str] = &[
    // Agility SDK exports are authoritative D3D12 evidence too.
    "D3D12CreateDevice", "D3D12SDKPath", "D3D12SDKVersion",
    "D3D11CreateDevice", "D3D10CreateDevice",
    "DirectDrawCreateEx", "DirectDrawCreate",
    "Direct3DCreate9", "Direct3DCreate8", "CreateDXGIFactory", "vkCreateInstance", "wglCreateContext",
];

fn choice(api: &str, label: &str) -> ApiChoice {
    ApiChoice { api: api.into(), label: label.into() }
}

fn api_from_names(imports: &[String]) -> Option<ApiChoice> {
    let has = |n: &str| imports.iter().any(|i| i == n);
    if has("d3d12.dll") { return Some(choice("dxgi", "DirectX 12")); }
    if has("d3d11.dll") { return Some(choice("dxgi", "DirectX 11")); }
    if has("d3d10.dll") || has("d3d10_1.dll") { return Some(choice("d3d10", "DirectX 10")); }
    if has("dxgi.dll") { return Some(choice("dxgi", "DirectX (DXGI)")); }
    if has("vulkan-1.dll") { return Some(choice("vulkan", "Vulkan")); }
    if has("d3d9.dll") { return Some(choice("d3d9", "DirectX 9")); }
    if has("d3d8.dll") { return Some(choice("d3d8", "DirectX 8")); }
    if has("ddraw.dll") { return Some(choice("ddraw", "DirectDraw")); }
    if has("opengl32.dll") { return Some(choice("opengl", "OpenGL")); }
    None
}

fn api_from_markers(file: &Path) -> Option<ApiChoice> {
    let m = pe::find_markers(file, API_MARKERS);
    let has = |s: &str| m.contains(s);
    if has("D3D12CreateDevice") || has("D3D12SDKPath") || has("D3D12SDKVersion") {
        return Some(choice("dxgi", "DirectX 12"));
    }
    if has("D3D11CreateDevice") { return Some(choice("dxgi", "DirectX 11")); }
    if has("D3D10CreateDevice") { return Some(choice("d3d10", "DirectX 10")); }
    if has("CreateDXGIFactory") { return Some(choice("dxgi", "DirectX (DXGI)")); }
    if has("Direct3DCreate9") { return Some(choice("d3d9", "DirectX 9")); }
    if has("Direct3DCreate8") { return Some(choice("d3d8", "DirectX 8")); }
    if has("DirectDrawCreateEx") || has("DirectDrawCreate") { return Some(choice("ddraw", "DirectDraw")); }
    if has("vkCreateInstance") { return Some(choice("vulkan", "Vulkan")); }
    if has("wglCreateContext") { return Some(choice("opengl", "OpenGL")); }
    None
}

fn api_from_file_name(file: &Path) -> Option<ApiChoice> {
    let name = file.file_name()?.to_string_lossy().to_lowercase();
    let t = |pat: &str| Regex::new(&format!(r"(?:^|[_-])(?:{pat})(?:[_-]|\.|$)")).unwrap().is_match(&name);
    if t("d3d12|dx12") { return Some(choice("dxgi", "DirectX 12")); }
    if t("d3d11|dx11") { return Some(choice("dxgi", "DirectX 11")); }
    if t("d3d10|dx10") { return Some(choice("d3d10", "DirectX 10")); }
    if t("d3d9|dx9") { return Some(choice("d3d9", "DirectX 9")); }
    if t("d3d8|dx8") { return Some(choice("d3d8", "DirectX 8")); }
    if t("ddraw") { return Some(choice("ddraw", "DirectDraw")); }
    if t("vulkan") { return Some(choice("vulkan", "Vulkan")); }
    if t("ogl|opengl") { return Some(choice("opengl", "OpenGL")); }
    None
}

/// Source/X-Ray/Ubisoft style dispatchers load their renderer from a module.
fn detect_engine_api(file: &Path) -> Option<(ApiChoice, String)> {
    let name = file.file_name()?.to_string_lossy().to_lowercase();
    let modules: &[&str] = match name.as_str() {
        "hl.exe" => &["hw.dll"],
        "hl2.exe" => &["bin/shaderapidx9.dll", "bin/engine.dll", "bin/x64/shaderapidx9.dll", "bin/x64/engine.dll"],
        "left4dead2.exe" => &["bin/shaderapidx9.dll", "bin/engine.dll"],
        "killingfloor.exe" => &["D3D9Drv.dll", "D3DDrv.dll", "OpenGLDrv.dll"],
        "farcry5.exe" => &["FC_m64.dll"],
        "watch_dogs.exe" => &["Disrupt_b64.dll"],
        "kingdomcome.exe" => &["WHGame.dll"],
        "xrengine.exe" => &["xrRender_R4.dll", "xrRender_R3.dll", "xrRender_R2.dll", "xrRender_R1.dll"],
        "portal2.exe" => &["bin/shaderapidx9.dll", "bin/x64/shaderapidx9.dll"],
        "csgo.exe" => &["bin/shaderapidx9.dll"],
        "garrysmod.exe" => &["bin/shaderapidx9.dll", "bin/win64/shaderapidx9.dll"],
        _ => return None,
    };
    let bits = pe::bitness(file);
    for rel in modules {
        let mut module = file.parent()?.to_path_buf();
        let mut ok = true;
        for part in rel.split('/') {
            match find_case_insensitive(&module, part) {
                Some(p) => module = p,
                None => { ok = false; break; }
            }
        }
        if !ok || pe::bitness(&module) != bits {
            continue;
        }
        if let Some(api) = api_from_names(&pe::imports(&module)).or_else(|| api_from_markers(&module)) {
            return Some((api, format!("engine-module:{rel}")));
        }
    }
    None
}

/// Three ways a game can reach Direct3D, in order of certainty: it imports
/// the API; it is a protected build that resolves it at runtime (only the
/// entry-point names survive); or the renderer lives in one of its own DLLs.
fn detect_api(file: &Path, imports: &[String]) -> Option<(ApiChoice, String)> {
    if let Some(a) = api_from_names(imports) {
        return Some((a, "imports".into()));
    }
    if let Some(a) = api_from_markers(file) {
        return Some((a, "strings".into()));
    }
    let dir = file.parent()?;
    let bits = pe::bitness(file);
    for name in imports.iter().take(80) {
        if name.contains(['\\', '/', ':']) {
            continue;
        }
        let Some(sibling) = find_case_insensitive(dir, name) else { continue };
        if pe::bitness(&sibling) != bits {
            continue;
        }
        if let Some(a) = api_from_names(&pe::imports(&sibling)).or_else(|| api_from_markers(&sibling)) {
            return Some((a, format!("module:{name}")));
        }
    }
    if let Some(a) = api_from_file_name(file) {
        return Some((a, "filename".into()));
    }
    detect_engine_api(file)
}

fn rdr2_renderer() -> ApiChoice {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let rel = Path::new("Rockstar Games").join("Red Dead Redemption 2").join("Settings").join("system.xml");
    let mut roots = vec![Path::new(&home).join("Documents"), Path::new(&home).join("OneDrive").join("Documents")];
    if let Ok(od) = std::env::var("OneDrive") {
        roots.push(Path::new(&od).join("Documents"));
    }
    let re = Regex::new(r"(?i)<API[^>]*>([^<]*)</API>").unwrap();
    for root in roots {
        let Ok(text) = fs::read_to_string(root.join(&rel)) else { continue };
        if let Some(c) = re.captures(&text) {
            if c[1].to_lowercase().contains("vulkan") {
                return choice("vulkan", "Vulkan");
            }
            return choice("dxgi", "DirectX 12");
        }
    }
    choice("dxgi", "DirectX 12")
}

fn playable_role_score(rel: &str) -> i32 {
    let rel = rel.to_lowercase();
    let mut score = 0;
    let t = |p: &str| Regex::new(p).unwrap().is_match(&rel);
    if t(r"(?:^|[\\/])singleplayer(?:[\\/]|$)") { score += 4; }
    if t(r"sp(?:[_-][^\\/]*)?\.exe$") { score += 2; }
    if t(r"(?:^|[\\/])(?:multiplayer|online)(?:[\\/]|$)") { score -= 4; }
    if t(r"mp(?:[_-][^\\/]*)?\.exe$") { score -= 2; }
    score
}

fn path_distance(a: &Path, b: &Path) -> usize {
    let l: Vec<String> = a.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let r: Vec<String> = b.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let common = l.iter().zip(r.iter()).take_while(|(x, y)| x == y).count();
    (l.len() - common) + (r.len() - common)
}

pub fn select_primary_dlss<'a>(files: &'a [DllFile], chosen: Option<&ExeCandidate>) -> Option<&'a DllFile> {
    let exact: Vec<&DllFile> = files.iter().filter(|f| f.name.eq_ignore_ascii_case("nvngx_dlss.dll")).collect();
    let legacy: Vec<&DllFile> = files
        .iter()
        .filter(|f| f.name.eq_ignore_ascii_case("nvngx.dll") || f.name.eq_ignore_ascii_case("_nvngx.dll"))
        .collect();
    let candidates = if exact.is_empty() { legacy } else { exact };
    let exe_dir = chosen.and_then(|c| c.path.parent());
    let score = |f: &DllFile| -> i64 {
        let mut v = 0i64;
        if f.version.is_some() { v += 10000; }
        if let Some(c) = chosen {
            if f.bitness == Some(c.bitness) { v += 2000; }
            let re = Regex::new(&format!(r"(?i)(?:^|[\\/])win{}(?:[\\/]|$)", c.bitness)).unwrap();
            if re.is_match(&f.rel) { v += 500; }
        }
        if let (Some(d), Some(p)) = (exe_dir, f.path.parent()) {
            v -= path_distance(p, d) as i64;
        }
        v
    };
    candidates.into_iter().max_by(|a, b| score(a).cmp(&score(b)).then_with(|| b.rel.cmp(&a.rel)))
}

/// Executables declared by an Xbox/GDK package, which may be encrypted.
fn xbox_executables(game_dir: &Path) -> Vec<(PathBuf, String, Option<u32>, bool)> {
    let mut configs = Vec::new();
    if let Some(c) = find_case_insensitive(game_dir, "MicrosoftGame.config") { configs.push(c); }
    if let Some(c) = find_case_insensitive(&game_dir.join("Content"), "MicrosoftGame.config") { configs.push(c); }
    let exe_re = Regex::new(r"(?is)<Executable\b([^>]*)/?\s*>").unwrap();
    let attr_re = Regex::new(r#"([\w:.-]+)\s*=\s*["']([^"']*)["']"#).unwrap();
    let mut found = Vec::new();
    for config in configs {
        let Ok(text) = fs::read_to_string(&config) else { continue };
        for m in exe_re.captures_iter(&text) {
            let mut name = None;
            let mut arch = String::new();
            for a in attr_re.captures_iter(&m[1]) {
                match a[1].to_lowercase().as_str() {
                    "name" => name = Some(a[2].to_string()),
                    "architecture" => arch = a[2].to_lowercase(),
                    _ => {}
                }
            }
            let Some(name) = name else { continue };
            if name.to_lowercase().ends_with("gamelaunchhelper.exe") { continue; }
            let full = config.parent().unwrap_or(game_dir).join(name.replace('/', "\\"));
            if !full.starts_with(game_dir) { continue; }
            let rel = rel_of(game_dir, &full);
            let bits = match arch.as_str() { "x64" => Some(64), "x86" => Some(32), _ => None };
            found.push((full, rel, bits, arch == "arm64"));
        }
    }
    found
}

/// A bounded search for the game's own renderer DLL, for an explanation
/// when no executable named an API.
fn renderer_module(game_dir: &Path, mut budget: usize) -> Option<String> {
    let mut queue = VecDeque::from([(game_dir.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        if budget == 0 { break; }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 4 { queue.push_back((e.path(), depth + 1)); }
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !name.to_lowercase().ends_with(".dll") || NOT_A_RENDERER.is_match(&name) { continue; }
            if budget == 0 { break; }
            budget -= 1;
            let p = e.path();
            if api_from_names(&pe::imports(&p)).or_else(|| api_from_markers(&p)).is_some() {
                return Some(name);
            }
        }
    }
    None
}

/// Renderer evidence from a DLL in an undetected executable's own folder.
fn renderer_beside(exes: &[ExeCandidate], mut budget: usize) -> Option<(ExeCandidate, ApiChoice, String)> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for e in exes {
        if let Some(d) = e.path.parent() {
            if !dirs.iter().any(|x| x == d) { dirs.push(d.to_path_buf()); }
        }
    }
    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.to_lowercase().ends_with(".dll") || NOT_A_RENDERER.is_match(&name) || NOT_OFFERED.is_match(&name) { continue; }
            if budget == 0 { return None; }
            budget -= 1;
            let file = e.path();
            let Some(api) = api_from_names(&pe::imports(&file)).or_else(|| api_from_markers(&file)) else { continue };
            let bits = pe::bitness(&file);
            let mut here: Vec<&ExeCandidate> = exes
                .iter()
                .filter(|x| x.path.parent() == Some(dir.as_path()) && Some(x.bitness) == bits)
                .collect();
            here.sort_by(|a, b| playable_role_score(&b.rel).cmp(&playable_role_score(&a.rel)).then(b.size.cmp(&a.size)));
            if let Some(exe) = here.first() {
                return Some(((*exe).clone(), api, name));
            }
        }
    }
    None
}

static ANTI_CHEAT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)easyanticheat|battleye|(?:^|[-_])(?:eac|be)launcher|eaanticheat").unwrap());

/// A shallow look for EasyAntiCheat / BattlEye / EA anti-cheat files.
pub fn anti_cheat_present(game_dir: &Path) -> bool {
    if game_dir.to_string_lossy().to_lowercase().replace(['_', '-', ' '], "").contains("arcraiders") {
        return true;
    }
    let mut queue = VecDeque::from([(game_dir.to_path_buf(), 0usize)]);
    let mut examined = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            if examined >= 2000 { return false; }
            examined += 1;
            let name = e.file_name().to_string_lossy().to_string();
            if ANTI_CHEAT.is_match(&name) { return true; }
            if depth < 2 && e.file_type().map(|t| t.is_dir()).unwrap_or(false) && !name.eq_ignore_ascii_case(BACKUP_DIR) {
                queue.push_back((e.path(), depth + 1));
            }
        }
    }
    false
}

pub fn read_install_state(game_dir: &Path) -> Option<InstallState> {
    let text = fs::read_to_string(game_dir.join(BACKUP_DIR).join("manifest.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let bitness = v["game"]["bitness"].as_u64();
    Some(InstallState {
        route: v["route"].as_str().map(String::from).unwrap_or_else(|| if bitness == Some(32) { "feeder".into() } else { "native".into() }),
        api: v["game"]["api"].as_str().map(String::from),
        exe: v["game"]["exe"].as_str().map(String::from),
        added: v["added"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        date: v["date"].as_str().map(String::from),
    })
}

pub fn scan_game(game_dir: &Path) -> GameScan {
    let mut exes: Vec<ExeCandidate> = Vec::new();
    let mut undetected: Vec<ExeCandidate> = Vec::new();
    let mut dlss_files = Vec::new();
    let mut streamline_files = Vec::new();
    let mut addon_present = None;
    let declared = xbox_executables(game_dir);
    let xbox_layout = !declared.is_empty() || game_dir.to_string_lossy().to_lowercase().contains("xboxgames");

    walk(game_dir, xbox_layout, |full, name, depth| {
        let lower = name.to_lowercase();
        let rel = rel_of(game_dir, full);
        if lower.ends_with(".exe") {
            if NOT_A_GAME.is_match(&lower) { return; }
            let size = fs::metadata(full).map(|m| m.len()).unwrap_or(0);
            let Some(bits) = pe::bitness(full) else { return };
            let detected = if lower == "rdr2.exe" {
                Some((rdr2_renderer(), "game-profile".to_string()))
            } else {
                detect_api(full, &pe::imports(full))
            };
            let mut c = ExeCandidate {
                path: full.to_path_buf(), rel, name: name.to_string(), size, depth,
                api: None, api_label: None, via: "undetected".into(), bitness: bits,
                dx12: false, declared: false, encrypted: false, api_choices: Vec::new(),
            };
            match detected {
                None => undetected.push(c),
                Some((api, via)) => {
                    c.dx12 = api.label == "DirectX 12";
                    c.api_choices = if lower == "rdr2.exe" {
                        vec![choice("dxgi", "DirectX 12"), choice("vulkan", "Vulkan")]
                    } else {
                        vec![api.clone()]
                    };
                    c.api = Some(api.api);
                    c.api_label = Some(api.label);
                    c.via = via;
                    exes.push(c);
                }
            }
        } else if DLSS_FILE.is_match(name) || STREAMLINE_FILE.is_match(name) {
            let item = DllFile {
                path: full.to_path_buf(), rel, name: name.to_string(),
                version: pe::file_version(full), bitness: pe::bitness(full), depth,
            };
            if STREAMLINE_FILE.is_match(name) { streamline_files.push(item) } else { dlss_files.push(item) }
        } else if lower.ends_with(".addon64") || lower.ends_with(".addon32") || lower.ends_with(".addon") {
            addon_present = Some(rel);
        }
    });

    for (path, rel, bits, arm64) in &declared {
        if !path.exists() || *arm64 { continue; }
        if let Some(c) = exes.iter_mut().find(|c| c.path.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy())) {
            c.declared = true;
            continue;
        }
        let (api, via) = detect_api(path, &pe::imports(path))
            .unwrap_or((choice("dxgi", "DirectX 11/12"), "MicrosoftGame.config".into()));
        let real_bits = pe::bitness(path);
        exes.push(ExeCandidate {
            path: path.clone(), rel: rel.clone(),
            name: path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            depth: rel.matches('\\').count(),
            dx12: api.label == "DirectX 12",
            api_choices: vec![api.clone()],
            api: Some(api.api), api_label: Some(api.label), via,
            bitness: real_bits.or(*bits).unwrap_or(64),
            declared: true, encrypted: real_bits.is_none(),
        });
    }

    // DX12 beats DX11, the launched role beats MP/online, shallow beats
    // buried, bigger beats smaller. Copies left by a backup are dropped.
    exes.sort_by(|a, b| {
        (b.declared as i32).cmp(&(a.declared as i32))
            .then((b.dx12 as i32).cmp(&(a.dx12 as i32)))
            .then(playable_role_score(&b.rel).cmp(&playable_role_score(&a.rel)))
            .then(a.depth.cmp(&b.depth))
            .then(b.size.cmp(&a.size))
    });
    let mut seen: HashSet<String> = HashSet::new();
    exes.retain(|e| seen.insert(e.name.to_lowercase()));

    // The renderer can be a DLL the engine loads by name (Prey, Titanfall 2).
    if exes.is_empty() && !undetected.is_empty() {
        if let Some((exe, api, module)) = renderer_beside(&undetected, 60) {
            seen.insert(exe.name.to_lowercase());
            exes.push(ExeCandidate {
                dx12: api.label == "DirectX 12",
                api_choices: vec![api.clone()],
                api: Some(api.api), api_label: Some(api.label),
                via: format!("renderer-module:{module}"),
                ..exe
            });
        }
    }

    // Executables that named no API at all are offered only once the folder
    // is known to be a game, and always after the detected ones.
    let is_game = !exes.is_empty() || !dlss_files.is_empty() || !streamline_files.is_empty();
    if is_game && !undetected.is_empty() {
        undetected.sort_by(|a, b| {
            playable_role_score(&b.rel).cmp(&playable_role_score(&a.rel)).then(a.depth.cmp(&b.depth)).then(b.size.cmp(&a.size))
        });
        let mut offered = 0;
        for e in undetected {
            if offered >= UNDETECTED_LIMIT || !seen.insert(e.name.to_lowercase()) { continue; }
            offered += 1;
            exes.push(e);
        }
    }

    let empty_reason = if exes.is_empty() {
        let top: Vec<String> = fs::read_dir(game_dir)
            .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect())
            .unwrap_or_default();
        let packed = top.iter().any(|f| f == "setup.exe")
            && top.iter().any(|f| Regex::new(r"\.(bin|rar|iso|zip|part\d+)$").unwrap().is_match(f));
        Some(if !declared.is_empty() || game_dir.to_string_lossy().to_lowercase().contains("windowsapps") {
            "xbox-protected"
        } else if packed {
            "installer"
        } else if !top.iter().any(|f| f.ends_with(".exe")) {
            "no-exe"
        } else if renderer_module(game_dir, 60).is_some() {
            "renderer-in-dll"
        } else {
            "no-graphics-exe"
        }.to_string())
    } else {
        None
    };

    let reshade = match exes.first().and_then(|e| e.path.parent()) {
        Some(d) => inspect_reshade(d),
        None => inspect_reshade(game_dir),
    };
    let backup = game_dir.join(BACKUP_DIR);
    GameScan {
        game_name: game_dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        game_dir: game_dir.to_path_buf(),
        exe_candidates: exes,
        dlss_files,
        streamline_files,
        addon_present,
        empty_reason,
        reshade,
        install: read_install_state(game_dir),
        has_backup: backup.join("manifest.json").exists(),
        pending_recovery: backup.join("pending-switch.json").exists(),
        anti_cheat: anti_cheat_present(game_dir),
    }
}

/// Shader Model 5.1 arrived with the Windows 10 SDK and the neural pass is
/// compiled as cs_5_1. A game-local D3DCompiler_47.dll older than that is
/// loaded ahead of System32's and silently compiles the pass to nothing.
pub fn old_shader_compiler(exe_dir: &Path) -> Option<(PathBuf, String)> {
    let file = find_case_insensitive(exe_dir, "D3DCompiler_47.dll")?;
    let version = pe::file_version(&file)?;
    let major: u32 = version.split('.').next()?.parse().ok()?;
    if major >= 10 { None } else { Some((file, version)) }
}

/// Mod Organizer "Stock Game" setups deploy and remove hooks at launch;
/// writing into them behind the manager's back is not safe.
pub fn managed_mod_root(game_dir: &Path, exe: Option<&Path>) -> Option<PathBuf> {
    let mut dir = exe.and_then(|e| e.parent()).unwrap_or(game_dir).to_path_buf();
    for _ in 0..5 {
        if find_case_insensitive(&dir, "ModOrganizer.exe").is_some() && find_case_insensitive(&dir, "Stock Game").is_some() {
            return Some(dir);
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }
    None
}
