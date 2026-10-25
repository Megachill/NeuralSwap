//! The setup window: shown when NeuralSwap-Setup.exe runs from outside the
//! install folder, and for `--uninstall` from Settings → Apps.

use super::theme::{self, *};
use super::widgets;
use crate::setup;
use egui::{vec2, Align, Layout, RichText};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Install,
    Uninstall,
}

enum Msg {
    Progress(f32, String),
    Done(Result<Option<PathBuf>, String>),
}

pub struct Installer {
    mode: Mode,
    desktop: bool,
    launch: bool,
    remove_settings: bool,
    running: bool,
    progress: f32,
    status: String,
    result: Option<Result<Option<PathBuf>, String>>,
    rx: Option<Receiver<Msg>>,
    mark: Option<egui::TextureHandle>,
    existing: Option<String>,
    has_payload: bool,
    dir: String,
    existing_dir: Option<PathBuf>,
}

pub fn run(mode: Mode) -> eframe::Result<()> {
    let icon = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok().map(|i| {
        let rgba = i.to_rgba8();
        egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
    });
    let title = if mode == Mode::Install { "NeuralSwap Setup" } else { "Uninstall NeuralSwap" };
    let mut viewport = egui::ViewportBuilder::default().with_title(title).with_inner_size([620.0, 430.0]).with_resizable(false);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mark = image::load_from_memory(include_bytes!("../../assets/mark-128.png")).ok().map(|i| {
                let rgba = i.to_rgba8();
                let img = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                cc.egui_ctx.load_texture("mark", img, egui::TextureOptions::LINEAR)
            });
            Ok(Box::new(Installer {
                mode,
                desktop: true,
                launch: true,
                remove_settings: false,
                running: false,
                progress: 0.0,
                status: String::new(),
                result: None,
                rx: None,
                mark,
                existing: setup::installed_version(),
                dir: setup::install_dir().display().to_string(),
                existing_dir: setup::existing_install_dir(),
                has_payload: std::env::current_exe().map(|e| crate::dist::appended_entry_count(&e) > 0).unwrap_or(false),
            }))
        }),
    )
}

impl Installer {
    fn start(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.running = true;
        let (mode, desktop, remove_settings) = (self.mode, self.desktop, self.remove_settings);
        let dir = setup::normalize_choice(std::path::Path::new(self.dir.trim()));
        self.dir = dir.display().to_string();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = match mode {
                Mode::Install => setup::install(&dir, desktop, &mut |f, s| {
                    let _ = tx.send(Msg::Progress(f, s));
                    ctx.request_repaint();
                })
                .map(Some),
                Mode::Uninstall => setup::uninstall(remove_settings).map(|_| None),
            };
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });
    }
}

impl eframe::App for Installer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    Msg::Progress(f, s) => {
                        self.progress = f;
                        self.status = s;
                    }
                    Msg::Done(r) => {
                        self.running = false;
                        if r.is_ok() {
                            self.progress = 1.0;
                            if self.mode == Mode::Uninstall {
                                self.status = "Removed. The folder is deleted when this window closes.".into();
                            }
                        }
                        self.result = Some(r);
                    }
                }
            }
        }
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(28))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(m) = &self.mark {
                    ui.add(egui::Image::from_texture(m).fit_to_exact_size(vec2(64.0, 64.0)));
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("Neural").font(theme::bold(30.0)));
                        ui.label(RichText::new("Swap").font(theme::bold(30.0)).color(GREEN_HI));
                    });
                    ui.label(RichText::new(format!("Version {}  ·  DLSS 5 neural rendering for your games", env!("CARGO_PKG_VERSION"))).color(MUTED));
                });
            });
            ui.add_space(18.0);
            match self.mode {
                Mode::Install => {
                    let text = match &self.existing {
                        Some(v) if v.as_str() != env!("CARGO_PKG_VERSION") => format!("Updates the installed version {v}. Settings, history and game backups are kept."),
                        Some(_) => "Repairs the existing installation. Settings, history and game backups are kept.".to_string(),
                        None if self.has_payload => "Installs NeuralSwap for your Windows account — no administrator rights needed. Includes the DLSS 5 files (~250 MB).".to_string(),
                        None => "Installs NeuralSwap for your Windows account — no administrator rights needed. The DLSS 5 files are not included: import them after installing (Settings → Payload folder → Import).".to_string(),
                    };
                    ui.label(text);
                    ui.add_space(6.0);
                    ui.add_enabled_ui(!self.running && self.result.is_none(), |ui| {
                        ui.label(RichText::new("Install folder").size(12.5).color(MUTED));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.dir).desired_width(430.0));
                            if widgets::ghost_button(ui, "Browse…").clicked() {
                                if let Some(p) = rfd::FileDialog::new().set_title("Choose where to install NeuralSwap").pick_folder() {
                                    self.dir = setup::normalize_choice(&p).display().to_string();
                                }
                            }
                        });
                        if let Some(old) = &self.existing_dir {
                            let chosen = setup::normalize_choice(std::path::Path::new(self.dir.trim()));
                            if chosen.to_string_lossy().to_lowercase() != old.to_string_lossy().to_lowercase() {
                                ui.label(RichText::new(format!("NeuralSwap will move here from {} (DLSS files included).", old.display())).size(12.0).color(AMBER));
                            }
                        }
                        ui.checkbox(&mut self.desktop, "Create a desktop shortcut");
                        ui.checkbox(&mut self.launch, "Start NeuralSwap when finished");
                    });
                }
                Mode::Uninstall => {
                    ui.label("Removes NeuralSwap, its DLSS 5 payload, the shortcuts and the Apps entry.");
                    ui.label(RichText::new("Games you installed DLSS 5 into are not changed. Restore them first if you want their original files back — or later with the original DLSS 5 Swapper, which reads the same backups.").size(12.5).color(AMBER));
                    ui.add_space(6.0);
                    ui.add_enabled_ui(!self.running && self.result.is_none(), |ui| {
                        ui.checkbox(&mut self.remove_settings, "Also delete settings, history and cached artwork");
                    });
                }
            }
            ui.add_space(14.0);
            if self.running || self.result.is_some() {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), egui::Sense::hover());
                ui.painter().rect_filled(r, theme::radius(4), BORDER);
                let mut fill = r;
                let frac = if self.mode == Mode::Uninstall && self.running { 0.5 } else { self.progress };
                fill.set_width(r.width() * frac.clamp(0.0, 1.0));
                ui.painter().rect_filled(fill, theme::radius(4), GREEN);
                ui.label(RichText::new(&self.status).size(12.5).color(MUTED));
            }
            match &self.result {
                Some(Ok(_)) => {
                    ui.label(RichText::new(if self.mode == Mode::Install { "✔ NeuralSwap is installed." } else { "✔ NeuralSwap was removed." }).color(GREEN_HI).strong());
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(format!("✖ {e}")).color(RED));
                    let needs_admin = e.starts_with("No write access");
                    ui.horizontal(|ui| {
                        if widgets::ghost_button(ui, "Choose another folder").clicked() {
                            self.result = None;
                        }
                        if needs_admin && widgets::ghost_button(ui, "Run setup as administrator").clicked() {
                            run_elevated();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                }
                None => {}
            }
            ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                ui.horizontal(|ui| {
                    match (&self.result, self.mode) {
                        (None, Mode::Install) => {
                            if widgets::primary_button(ui, if self.running { "Installing…" } else { "Install" }, !self.running, vec2(170.0, 42.0)).clicked() {
                                self.start(&ctx);
                            }
                        }
                        (None, Mode::Uninstall) => {
                            if widgets::danger_button(ui, if self.running { "Removing…" } else { "Uninstall" }, !self.running, vec2(170.0, 42.0)).clicked() {
                                self.start(&ctx);
                            }
                        }
                        (Some(Ok(Some(exe))), Mode::Install) => {
                            if widgets::primary_button(ui, if self.launch { "Start NeuralSwap" } else { "Finish" }, true, vec2(170.0, 42.0)).clicked() {
                                if self.launch {
                                    let _ = std::process::Command::new(exe).spawn();
                                }
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        _ => {
                            if widgets::ghost_button(ui, "Close").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                    }
                    if self.result.is_none() && !self.running && widgets::ghost_button(ui, "Cancel").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });
    }
}

fn run_elevated() {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let Ok(exe) = std::env::current_exe() else { return };
    let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (w("runas"), w(&exe.to_string_lossy()));
    unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
}
