//! The desktop app: one native window, egui rendered on the GPU. All slow
//! work (launcher discovery, scanning, artwork, installs) runs on worker
//! threads and reports back over a channel.

pub mod installer;
mod pages;
mod sheet;
pub mod theme;
mod widgets;

use crate::i18n::t;
use crate::art;
use crate::core::apply::{self, InstallConfig, Route};
use crate::core::guards::{self, Gpu};
use crate::core::history::{HistoryRow, HistoryStore};
use crate::core::payload::{self, Payload};
use crate::core::scan::{self, ExeCandidate, GameScan};
use crate::core::Event;
use crate::library::{self, LibraryGame};
use crate::state::{data_dir, game_key, ArtMeta, ScanCache, Settings};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const APP_NAME: &str = "NeuralSwap";

pub fn run() -> eframe::Result<()> {
    let icon = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok().map(|i| {
        let rgba = i.to_rgba8();
        egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([1440.0, 920.0])
        .with_min_inner_size([980.0, 640.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Games,
    Addons,
    History,
    Settings,
    About,
}

pub struct GameEntry {
    pub lib: LibraryGame,
    pub key: String,
    pub scan: Option<GameScan>,
    pub scanning: bool,
}

/// An executable with the user's rendering-API override applied.
#[derive(Clone)]
pub struct Target {
    pub exe: ExeCandidate,
    pub api: Option<String>,
    pub api_label: Option<String>,
}

pub const API_CHOICES: &[(&str, &str, &str)] = &[
    ("d3d12", "dxgi", "DirectX 12"),
    ("d3d11", "dxgi", "DirectX 11"),
    ("d3d9", "d3d9", "DirectX 9"),
    ("vulkan", "vulkan", "Vulkan"),
    ("opengl", "opengl", "OpenGL"),
];

impl GameEntry {
    pub fn target(&self, settings: &Settings) -> Option<Target> {
        let scan = self.scan.as_ref()?;
        let prefs = settings.per_game.get(&self.key);
        let exe = scan.candidate(prefs.and_then(|p| p.exe_rel.as_deref()))?.clone();
        let choice = prefs.and_then(|p| p.api.clone()).unwrap_or_else(|| "auto".into());
        let (api, api_label) = match API_CHOICES.iter().find(|c| c.0 == choice) {
            Some((_, api, label)) => (Some(api.to_string()), Some(label.to_string())),
            None => (exe.api.clone(), exe.api_label.clone()),
        };
        Some(Target { exe, api, api_label })
    }

    pub fn routes(&self, settings: &Settings) -> Vec<Route> {
        self.target(settings)
            .map(|t| apply::routes_for(t.exe.bitness, t.api.as_deref(), t.api_label.as_deref()))
            .unwrap_or_default()
    }

    pub fn installed_route(&self) -> Option<&str> {
        self.scan.as_ref().and_then(|s| s.install.as_ref()).map(|i| i.route.as_str())
    }

    pub fn dlss_version(&self, settings: &Settings) -> Option<String> {
        let scan = self.scan.as_ref()?;
        let t = self.target(settings);
        scan.primary_dlss(t.as_ref().map(|t| &t.exe)).and_then(|f| f.version.clone())
    }

    pub fn ready(&self, settings: &Settings) -> bool {
        self.installed_route().is_none() && !self.routes(settings).is_empty()
    }
}

#[derive(Default)]
pub struct Filters {
    pub search: String,
    pub api: Option<String>,
    pub dlss: DlssFilter,
    pub addon: AddonFilter,
}

#[derive(Default, Clone, PartialEq)]
pub enum DlssFilter {
    #[default]
    All,
    Ready,
    Present,
    None,
    Installed,
    Version(String),
}

#[derive(Default, Clone, Copy, PartialEq)]
pub enum AddonFilter {
    #[default]
    Any,
    Installed,
    NotInstalled,
}

pub enum JobKind {
    Install,
    Restore,
}

pub struct Job {
    pub key: String,
    pub kind: JobKind,
}

pub struct Sheet {
    pub key: String,
    pub log: Vec<Event>,
    pub confirm_anticheat: bool,
    pub warnings_open: bool,
    pub last_error: Option<&'static str>,
}

enum Msg {
    Library(Vec<LibraryGame>),
    Scanned(String, Box<GameScan>),
    Art { key: String, meta: Option<ArtMeta>, cover: Option<ColorImage> },
    Hero { key: String, image: Option<ColorImage> },
    JobEvent(Event),
    JobDone { key: String, error: Option<crate::core::Error> },
    Gpu(Option<Vec<Gpu>>),
    UpdateChecked { manual: bool, result: Result<Option<crate::update::Available>, String> },
    UpdateProgress(f32, String),
    UpdateDone(Result<PathBuf, String>),
    ImportProgress(f32, String),
    ImportDone(Result<PathBuf, String>),
    /// A core zip holds builds we do not recognise: ask before installing.
    CoreNeedsConfirm(PathBuf, String),
    Magpie(Option<crate::magpie::Magpie>),
}

#[derive(Clone)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Box<crate::update::Available>),
    Applying(f32, String),
    Failed(String),
}

enum ArtJob {
    Cover { key: String, name: String, appid: Option<u32>, poster: Option<PathBuf>, meta: Option<ArtMeta>, fetch: bool },
    Hero { key: String, appid: u32 },
}

pub struct App {
    pub settings: Settings,
    cache: ScanCache,
    pub payload: Option<Payload>,
    pub games: Vec<GameEntry>,
    pub page: Page,
    pub filters: Filters,
    pub covers: HashMap<String, TextureHandle>,
    pub heroes: HashMap<String, TextureHandle>,
    art_requested: HashSet<String>,
    hero_requested: HashSet<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    scan_tx: Sender<(String, PathBuf)>,
    art_tx: Sender<ArtJob>,
    pub sheet: Option<Sheet>,
    pub job: Option<Job>,
    pub history: Vec<HistoryRow>,
    history_store: HistoryStore,
    pub gpu: Option<Vec<Gpu>>,
    pub logo: Option<TextureHandle>,
    pub discovering: bool,
    pub toast: Option<(String, Instant)>,
    settings_dirty: bool,
    /// `--open <name>`: open that game's sheet once the library is loaded.
    pending_open: Option<String>,
    pub update: UpdateState,
    /// Payload import in progress: (fraction, status).
    pub importing: Option<(f32, String)>,
    pub core_link: String,
    pub core_sha: String,
    pub core_confirm: Option<(PathBuf, String)>,
    /// Some(None) once a search found nothing; None while it runs.
    pub magpie: Option<Option<crate::magpie::Magpie>>,
    ctx: egui::Context,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        theme::install(ctx);
        let (tx, rx) = channel();
        let settings = Settings::load();
        crate::i18n::init(settings.language.as_deref());
        let cache = ScanCache::load();
        let payload = payload::locate(settings.payload_dir.as_deref());

        // Scan workers share one queue.
        let (scan_tx, scan_rx) = channel::<(String, PathBuf)>();
        let scan_rx = Arc::new(Mutex::new(scan_rx));
        for _ in 0..4 {
            let rx = scan_rx.clone();
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let next = rx.lock().unwrap().recv();
                let Ok((key, dir)) = next else { break };
                let result = scan::scan_game(&dir);
                if tx.send(Msg::Scanned(key, Box::new(result))).is_err() {
                    break;
                }
                ctx.request_repaint();
            });
        }

        // Artwork runs on one thread: the Steam store is rate limited anyway.
        let (art_tx, art_rx) = channel::<ArtJob>();
        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                while let Ok(job) = art_rx.recv() {
                    let msg = match job {
                        ArtJob::Cover { key, name, appid, poster, meta, fetch } => {
                            let meta_out = match meta {
                                Some(m) => Some(m),
                                None if fetch => Some(art::look_up(&name, appid)),
                                None => None,
                            };
                            let id = meta_out.as_ref().and_then(|m| m.appid).or(appid);
                            let path = poster.filter(|p| p.exists()).or_else(|| if fetch { id.and_then(art::cover) } else { None });
                            let cover = path.and_then(|p| art::decode(&p, 360, 540));
                            Msg::Art { key, meta: meta_out, cover }
                        }
                        ArtJob::Hero { key, appid } => Msg::Hero { image: art::hero(appid).and_then(|p| art::decode(&p, 1600, 520)), key },
                    };
                    if tx.send(msg).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }

        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Gpu(guards::gpu_info()));
                ctx.request_repaint();
            });
        }

        let logo = image::load_from_memory(include_bytes!("../../assets/mark-128.png")).ok().map(|i| {
            let rgba = i.to_rgba8();
            let img = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
            ctx.load_texture("logo", img, TextureOptions::LINEAR)
        });

        let history_store = HistoryStore { file: data_dir().join("history.jsonl") };
        let mut app = App {
            settings,
            cache,
            payload,
            games: Vec::new(),
            page: Page::Games,
            filters: Filters::default(),
            covers: HashMap::new(),
            heroes: HashMap::new(),
            art_requested: HashSet::new(),
            hero_requested: HashSet::new(),
            tx,
            rx,
            scan_tx,
            art_tx,
            sheet: None,
            job: None,
            history: Vec::new(),
            history_store,
            gpu: None,
            logo,
            discovering: false,
            toast: None,
            settings_dirty: false,
            pending_open: None,
            update: UpdateState::Idle,
            importing: None,
            core_link: String::new(),
            core_sha: String::new(),
            core_confirm: None,
            magpie: None,
            ctx: ctx.clone(),
        };
        app.history = app.history_store.list(&[]);
        let args: Vec<String> = std::env::args().collect();
        app.pending_open = args.iter().position(|a| a == "--open").and_then(|i| args.get(i + 1)).map(|s| s.to_lowercase());
        if let Some(page) = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)) {
            app.page = match page.as_str() {
                "home" => Page::Home,
                "addons" => Page::Addons,
                "history" => Page::History,
                "settings" => Page::Settings,
                "about" => Page::About,
                _ => Page::Games,
            };
        }
        app.discover();
        app.detect_magpie();
        if app.settings.auto_update_check {
            app.check_updates(false);
        }
        app
    }

    pub fn update_repo(&self) -> Option<String> {
        self.settings
            .update_repo
            .clone()
            .filter(|r| !r.trim().is_empty())
            .or_else(|| crate::update::DEFAULT_REPO.map(String::from))
    }

    pub fn check_updates(&mut self, manual: bool) {
        let Some(repo) = self.update_repo() else {
            if manual {
                self.update = UpdateState::Failed("Set the GitHub repository first".into());
            }
            return;
        };
        if matches!(self.update, UpdateState::Checking | UpdateState::Applying(..)) {
            return;
        }
        self.update = UpdateState::Checking;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let payload = self.payload.as_ref().map(|p| p.root.clone());
        std::thread::spawn(move || {
            let result = crate::update::check(&repo, payload.as_deref());
            let _ = tx.send(Msg::UpdateChecked { manual, result });
            ctx.request_repaint();
        });
    }

    /// Imports DLSS 5 files from a NeuralSwap-payload.zip or a payload folder
    /// into `payload` beside the executable, then switches to it.
    pub fn import_payload(&mut self, src: PathBuf) {
        if self.importing.is_some() || self.job.is_some() {
            return;
        }
        let Some(dest) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("payload"))) else { return };
        self.importing = Some((0.0, t("Starting import").into()));
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut progress = |f: f32, t: String| {
                let _ = tx.send(Msg::ImportProgress(f, t));
                ctx.request_repaint();
            };
            let result = import_into(&src, &dest, &mut progress).map(|_| dest);
            let _ = tx.send(Msg::ImportDone(result));
            ctx.request_repaint();
        });
    }

    /// Looks for Magpie on a worker thread (it may scan a few folders).
    pub fn detect_magpie(&mut self) {
        self.magpie = None;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let setting = self.settings.magpie_path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Magpie(crate::magpie::detect(setting.as_deref())));
            ctx.request_repaint();
        });
    }

    pub fn locate_magpie(&mut self) {
        if let Some(f) = rfd::FileDialog::new().set_title(t("Locate Magpie.exe")).add_filter("Magpie", &["exe"]).pick_file() {
            self.settings.magpie_path = Some(f);
            self.mark_settings_dirty();
            self.detect_magpie();
        }
    }

    pub fn open_magpie(&mut self) {
        match self.magpie.clone().flatten() {
            Some(m) => match crate::magpie::launch(&m) {
                Ok(()) => self.toast(t("Magpie started. Switch the game to borderless or windowed, then press Magpie's scale hotkey.")),
                Err(e) => self.toast(tr!("Could not start Magpie: {}", e)),
            },
            None => {
                let _ = open::that(crate::magpie::RELEASES_URL);
            }
        }
    }

    /// Where downloaded / imported DLSS 5 files are assembled.
    pub fn payload_target(&self) -> PathBuf {
        std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("payload"))).unwrap_or_else(|| PathBuf::from("payload"))
    }

    fn run_setup(&mut self, work: impl FnOnce(&mut dyn FnMut(f32, String)) -> Result<(), SetupError> + Send + 'static) {
        if self.importing.is_some() || self.job.is_some() {
            return;
        }
        self.core_confirm = None;
        self.importing = Some((0.0, t("Starting").into()));
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let dest = self.payload_target();
        std::thread::spawn(move || {
            let mut progress = |f: f32, t: String| {
                let _ = tx.send(Msg::ImportProgress(f, t));
                ctx.request_repaint();
            };
            let msg = match work(&mut progress) {
                Ok(()) => Msg::ImportDone(Ok(dest)),
                Err(SetupError::Unknown(zip, why)) => Msg::CoreNeedsConfirm(zip, why),
                Err(SetupError::Failed(e)) => Msg::ImportDone(Err(e)),
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    pub fn download_official(&mut self) {
        let dest = self.payload_target();
        self.run_setup(move |p| crate::components::download_official(&dest, p).map_err(SetupError::Failed));
    }

    /// Core files from a link (`zip` None) or a local zip.
    pub fn import_core(&mut self, zip: Option<PathBuf>, allow_unknown: bool) {
        let dest = self.payload_target();
        let link = self.core_link.trim().to_string();
        let sha = self.core_sha.trim().to_string();
        self.run_setup(move |p| {
            let zip = match zip {
                Some(z) => z,
                None => crate::components::download_link(&link, p).map_err(SetupError::Failed)?,
            };
            p(0.95, "Checking the core files".into());
            match crate::components::import_core(&zip, &dest, allow_unknown, Some(&sha)) {
                Ok(_) => Ok(()),
                Err(e) if e.starts_with("unrecognised build") => Err(SetupError::Unknown(zip, e)),
                Err(e) => Err(SetupError::Failed(e)),
            }
        });
    }

    pub fn apply_update(&mut self) {
        let UpdateState::Available(av) = self.update.clone() else { return };
        if self.job.is_some() {
            self.toast(t("Wait for the running install to finish first."));
            return;
        }
        self.update = UpdateState::Applying(0.0, t("Starting download").into());
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let payload = self.payload.as_ref().map(|p| p.root.clone());
        std::thread::spawn(move || {
            let mut progress = |f: f32, t: String| {
                let _ = tx.send(Msg::UpdateProgress(f, t));
                ctx.request_repaint();
            };
            let result = crate::update::apply(&av, payload.as_deref(), &mut progress);
            let _ = tx.send(Msg::UpdateDone(result));
            ctx.request_repaint();
        });
    }

    /// Re-reads every launcher, then rescans every game in the background.
    pub fn discover(&mut self) {
        if self.discovering {
            return;
        }
        self.discovering = true;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let (manual, folders, excluded) = (self.settings.manual_games.clone(), self.settings.folders.clone(), self.settings.excluded.clone());
        std::thread::spawn(move || {
            let games = library::discover(&manual, &folders, &excluded);
            let _ = tx.send(Msg::Library(games));
            ctx.request_repaint();
        });
    }

    pub fn rescan(&mut self, key: &str) {
        if let Some(g) = self.games.iter_mut().find(|g| g.key == key) {
            g.scanning = true;
            let _ = self.scan_tx.send((g.key.clone(), g.lib.dir.clone()));
        }
    }

    pub fn reload_payload(&mut self) {
        self.payload = payload::locate(self.settings.payload_dir.as_deref());
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    pub fn game(&self, key: &str) -> Option<&GameEntry> {
        self.games.iter().find(|g| g.key == key)
    }

    pub fn open_sheet(&mut self, key: &str) {
        self.sheet = Some(Sheet { key: key.to_string(), log: Vec::new(), confirm_anticheat: false, warnings_open: true, last_error: None });
        self.request_hero(key);
        if let Some(idx) = self.games.iter().position(|g| g.key == key) {
            self.ensure_cover(idx);
        }
        if self.job.is_none() {
            self.rescan(key);
        }
    }

    fn request_hero(&mut self, key: &str) {
        if self.hero_requested.contains(key) || !self.settings.fetch_art {
            return;
        }
        let appid = self.game(key).and_then(|g| g.lib.steam_appid.or_else(|| self.settings.art.get(key).and_then(|m| m.appid)));
        if let Some(appid) = appid {
            self.hero_requested.insert(key.to_string());
            let _ = self.art_tx.send(ArtJob::Hero { key: key.to_string(), appid });
        }
    }

    pub fn ensure_cover(&mut self, idx: usize) {
        let g = &self.games[idx];
        if self.covers.contains_key(&g.key) || self.art_requested.contains(&g.key) {
            return;
        }
        self.art_requested.insert(g.key.clone());
        let meta = self.settings.art.get(&g.key).filter(|m| m.looked_up).cloned();
        let _ = self.art_tx.send(ArtJob::Cover {
            key: g.key.clone(),
            name: g.lib.name.clone(),
            appid: g.lib.steam_appid,
            poster: g.lib.poster.clone(),
            meta,
            fetch: self.settings.fetch_art,
        });
    }

    pub fn chosen_route(&self, key: &str) -> Option<Route> {
        let g = self.game(key)?;
        let routes = g.routes(&self.settings);
        let picked = self.settings.per_game.get(key).and_then(|p| p.route.as_deref()).and_then(Route::from_key);
        if let Some(r) = picked.filter(|r| routes.contains(r)) {
            return Some(r);
        }
        let scan = g.scan.as_ref()?;
        let t = g.target(&self.settings);
        apply::recommended_route(&routes, scan.native_dlss_present(t.as_ref().map(|t| &t.exe)), g.installed_route())
    }

    /// Runs an install on a worker thread.
    pub fn start_install(&mut self, key: &str, acknowledged: bool) {
        if self.job.is_some() {
            return;
        }
        let Some(route) = self.chosen_route(key) else { return };
        let Some(g) = self.game(key) else { return };
        let Some(target) = g.target(&self.settings) else { return };
        let Some(payload) = self.payload.clone() else { return };
        let cfg = InstallConfig {
            game_dir: g.lib.dir.clone(),
            exe_path: target.exe.path.clone(),
            api: target.api.clone().unwrap_or_else(|| "dxgi".into()),
            api_label: target.api_label.clone(),
            bitness: target.exe.bitness,
            route,
            anti_cheat_acknowledged: acknowledged,
            payload,
            companions: self.settings.companions.iter().filter(|c| c.exists()).cloned().collect(),
        };
        let name = g.lib.name.clone();
        if let Some(s) = self.sheet.as_mut() {
            s.log.clear();
            s.last_error = None;
            s.log.push(Event::info(tr!("Installing into {} via {} …", name, t(route.label()))));
        }
        self.job = Some(Job { key: key.to_string(), kind: JobKind::Install });
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let key = key.to_string();
        let store = HistoryStore { file: self.history_store.file.clone() };
        std::thread::spawn(move || {
            let mut log = |e: Event| {
                let _ = tx.send(Msg::JobEvent(e));
                ctx.request_repaint();
            };
            let error = match apply::install(&cfg, &mut log) {
                Ok(m) => {
                    store.record(&cfg.game_dir, Some(&m), "install", &name);
                    None
                }
                Err(e) => Some(e),
            };
            let _ = tx.send(Msg::JobDone { key, error });
            ctx.request_repaint();
        });
    }

    pub fn start_restore(&mut self, key: &str) {
        if self.job.is_some() {
            return;
        }
        let Some(g) = self.game(key) else { return };
        let dir = g.lib.dir.clone();
        let name = g.lib.name.clone();
        if let Some(s) = self.sheet.as_mut() {
            s.log.clear();
            s.last_error = None;
            s.log.push(Event::info(tr!("Restoring {} …", name)));
        }
        self.job = Some(Job { key: key.to_string(), kind: JobKind::Restore });
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let key = key.to_string();
        let store = HistoryStore { file: self.history_store.file.clone() };
        std::thread::spawn(move || {
            let mut log = |e: Event| {
                let _ = tx.send(Msg::JobEvent(e));
                ctx.request_repaint();
            };
            let error = match apply::restore(&dir, &mut log) {
                Ok(m) => {
                    store.record(&dir, m.as_ref(), if m.is_some() { "restore" } else { "recovery" }, &name);
                    None
                }
                Err(e) => Some(e),
            };
            let _ = tx.send(Msg::JobDone { key, error });
            ctx.request_repaint();
        });
    }

    fn pump(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Library(list) => {
                    self.discovering = false;
                    let mut entries = Vec::new();
                    for lib in list {
                        let key = game_key(&lib.dir);
                        let scan = self
                            .games
                            .iter()
                            .find(|g| g.key == key)
                            .and_then(|g| g.scan.clone())
                            .or_else(|| self.cache.scans.get(&key).cloned());
                        entries.push(GameEntry { lib, key, scan, scanning: true });
                    }
                    entries.sort_by(|a, b| a.lib.name.to_lowercase().cmp(&b.lib.name.to_lowercase()));
                    // Unscanned games first so the grid fills in quickly.
                    let mut order: Vec<&GameEntry> = entries.iter().collect();
                    order.sort_by_key(|g| g.scan.is_some());
                    for g in order {
                        let _ = self.scan_tx.send((g.key.clone(), g.lib.dir.clone()));
                    }
                    self.games = entries;
                    self.refresh_history();
                }
                Msg::Scanned(key, scan) => {
                    if let Some(g) = self.games.iter_mut().find(|g| g.key == key) {
                        g.scan = Some((*scan).clone());
                        g.scanning = false;
                    }
                    self.cache.scans.insert(key, *scan);
                    if self.games.iter().all(|g| !g.scanning) {
                        self.cache.save();
                    }
                }
                Msg::Art { key, meta, cover } => {
                    if let Some(m) = meta {
                        if self.settings.art.get(&key).map(|o| !o.looked_up).unwrap_or(true) {
                            self.settings.art.insert(key.clone(), m);
                            self.settings_dirty = true;
                        }
                    }
                    if let Some(img) = cover {
                        let tex = self.ctx.load_texture(format!("cover:{key}"), img, TextureOptions::LINEAR);
                        self.covers.insert(key.clone(), tex);
                    }
                    if self.sheet.as_ref().map(|s| s.key == key).unwrap_or(false) {
                        self.request_hero(&key);
                    }
                }
                Msg::Hero { key, image } => {
                    if let Some(img) = image {
                        let tex = self.ctx.load_texture(format!("hero:{key}"), img, TextureOptions::LINEAR);
                        self.heroes.insert(key, tex);
                    }
                }
                Msg::JobEvent(e) => {
                    if let Some(s) = self.sheet.as_mut() {
                        s.log.push(e);
                    }
                }
                Msg::JobDone { key, error } => {
                    self.job = None;
                    match error {
                        None => self.toast(t("Done.")),
                        Some(e) => {
                            if let Some(s) = self.sheet.as_mut().filter(|s| s.key == key) {
                                s.last_error = Some(e.code);
                            }
                            self.toast(tr!("Failed: {}", t(e.message())));
                        }
                    }
                    self.rescan(&key);
                    self.refresh_history();
                }
                Msg::Gpu(rows) => self.gpu = rows,
                Msg::UpdateChecked { manual, result } => {
                    self.update = match result {
                        Ok(Some(av)) => UpdateState::Available(Box::new(av)),
                        Ok(None) => UpdateState::UpToDate,
                        // A failed background check stays quiet; a manual one says why.
                        Err(e) if manual => UpdateState::Failed(e),
                        Err(_) => UpdateState::Idle,
                    };
                }
                Msg::UpdateProgress(f, t) => self.update = UpdateState::Applying(f, t),
                Msg::ImportProgress(f, t) => self.importing = Some((f, t)),
                Msg::Magpie(found) => self.magpie = Some(found),
                Msg::CoreNeedsConfirm(zip, why) => {
                    self.importing = None;
                    self.core_confirm = Some((zip, why));
                }
                Msg::ImportDone(result) => {
                    self.importing = None;
                    match result {
                        Ok(dest) => {
                            self.settings.payload_dir = None;
                            self.settings_dirty = true;
                            self.reload_payload();
                            let here = self.payload.as_ref().map(|p| p.root == dest).unwrap_or(false);
                            let complete = self.payload.as_ref().map(|p| p.complete()).unwrap_or(false);
                            self.toast(match (complete, here) {
                                (true, true) => t("DLSS 5 files ready."),
                                (true, false) => t("Done. Another complete payload folder is in use. Check Settings."),
                                (false, _) => t("Done. Still missing: see the DLSS 5 files card."),
                            });
                        }
                        Err(e) => self.toast(tr!("Import failed: {}", e)),
                    }
                }
                Msg::UpdateDone(result) => match result {
                    Ok(exe) => {
                        self.settings.save();
                        self.cache.save();
                        let _ = std::process::Command::new(exe).spawn();
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => self.update = UpdateState::Failed(e),
                },
            }
        }
    }

    pub fn refresh_history(&mut self) {
        let folders: Vec<(PathBuf, String)> = self.games.iter().map(|g| (g.lib.dir.clone(), g.lib.name.clone())).collect();
        self.history = self.history_store.list(&folders);
    }

    pub fn add_game_folder(&mut self, dir: PathBuf) {
        let k = game_key(&dir);
        self.settings.excluded.retain(|e| game_key(e) != k);
        if !self.settings.manual_games.iter().any(|g| game_key(g) == k) {
            self.settings.manual_games.push(dir);
        }
        self.settings_dirty = true;
        self.discover();
    }

    pub fn add_library_folder(&mut self, dir: PathBuf) {
        let k = game_key(&dir);
        if !self.settings.folders.iter().any(|g| game_key(g) == k) {
            self.settings.folders.push(dir);
        }
        self.settings_dirty = true;
        self.discover();
    }

    pub fn exclude(&mut self, key: &str) {
        if let Some(g) = self.game(key) {
            let dir = g.lib.dir.clone();
            self.settings.manual_games.retain(|m| game_key(m) != key);
            self.settings.excluded.push(dir);
            self.games.retain(|g| g.key != key);
            self.settings_dirty = true;
        }
    }

    pub fn restart_as_admin(&self) {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let Ok(exe) = std::env::current_exe() else { return };
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (verb, file) = (w("runas"), w(&exe.to_string_lossy()));
        let rc = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if rc as isize > 32 {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump();
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            if p.is_dir() {
                self.add_game_folder(p);
            } else if let Some(parent) = p.parent() {
                self.add_game_folder(parent.to_path_buf());
            }
        }
        if let Some(name) = self.pending_open.clone() {
            if let Some(key) = self.games.iter().find(|g| g.lib.name.to_lowercase().contains(&name)).map(|g| g.key.clone()) {
                self.pending_open = None;
                self.open_sheet(&key);
            }
        }
        pages::sidebar(self, ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin { left: 22, right: 22, top: 18, bottom: 12 }))
            .show(ui, |ui| match self.page {
                Page::Home => pages::home(self, ui),
                Page::Games => pages::games(self, ui),
                Page::Addons => pages::addons(self, ui),
                Page::History => pages::history(self, ui),
                Page::Settings => pages::settings(self, ui),
                Page::About => pages::about(self, ui),
            });
        if self.sheet.is_some() {
            let ctx = ui.ctx().clone();
            sheet::show(self, &ctx);
        }
        let ctx = ui.ctx().clone();
        widgets::toast(self, &ctx);
        if self.settings_dirty {
            self.settings.save();
            self.settings_dirty = false;
        }
        if self.job.is_some() || self.discovering || self.importing.is_some() || self.games.iter().any(|g| g.scanning) || matches!(self.update, UpdateState::Checking | UpdateState::Applying(..)) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    fn on_exit(&mut self) {
        self.settings.save();
        self.cache.save();
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path, done: &mut u64, total: u64, progress: &mut dyn FnMut(f32, String)) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &target, done, total, progress)?;
        } else {
            *done += std::fs::copy(e.path(), &target)?;
            progress(*done as f32 / total.max(1) as f32, tr!("Copying DLSS 5 files  {} / {} MB", *done >> 20, total >> 20));
        }
    }
    Ok(())
}

/// Stages the import beside `dest`, checks it really is a payload, then swaps.
pub fn import_into(src: &std::path::Path, dest: &std::path::Path, progress: &mut dyn FnMut(f32, String)) -> Result<(), String> {
    let staging = PathBuf::from(format!("{}.new", dest.display()));
    let _ = std::fs::remove_dir_all(&staging);
    if src.is_dir() {
        if payload::scan_payload(src).is_none() {
            return Err("that folder has no streamline DLSS files".into());
        }
        let total = crate::dist::payload_size(src);
        copy_tree(src, &staging, &mut 0, total, progress).map_err(|e| e.to_string())?;
    } else {
        crate::dist::extract(src, false, &staging, &mut |d, t| progress(d as f32 / t.max(1) as f32, tr!("Extracting DLSS 5 files  {} / {} MB", d >> 20, t >> 20)))
            .map_err(|e| e.to_string())?;
    }
    if payload::scan_payload(&staging).is_none() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("not a DLSS 5 payload (no streamline DLSS files inside)".into());
    }
    let old = PathBuf::from(format!("{}.old", dest.display()));
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(dest, &old).map_err(|e| format!("could not replace the current payload: {e}"))?;
    }
    std::fs::rename(&staging, dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

pub enum SetupError {
    Failed(String),
    Unknown(PathBuf, String),
}
