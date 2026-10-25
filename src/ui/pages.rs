//! The sidebar and every page except the game sheet.

use super::theme::{self, *};
use super::widgets::{self, *};
use super::{AddonFilter, App, DlssFilter, Page, UpdateState, APP_NAME};
use crate::state::game_key;
use egui::{vec2, Align, Color32, Layout, RichText, Stroke, Ui};

const NAV: &[(Page, &str, &str)] = &[
    (Page::Home, "🏠", "Home"),
    (Page::Games, "🎮", "Games"),
    (Page::Addons, "🔌", "Add-ons"),
    (Page::History, "🕘", "History"),
    (Page::Settings, "⚙", "Settings"),
    (Page::About, "ℹ", "About"),
];

pub fn sidebar(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("sidebar")
        .resizable(false)
        .exact_size(230.0)
        .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 14, right: 14, top: 18, bottom: 14 }).stroke(Stroke::new(1.0, BORDER)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(mark) = &app.logo {
                    ui.add(egui::Image::from_texture(mark).fit_to_exact_size(vec2(46.0, 46.0)));
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("Neural").font(theme::bold(23.0)).color(TEXT));
                        ui.label(RichText::new("Swap").font(theme::bold(23.0)).color(GREEN_HI));
                    });
                    ui.label(RichText::new("DLSS 5 · NEURAL RENDERING").size(9.5).color(MUTED));
                });
            });
            ui.add_space(22.0);
            for (page, icon, label) in NAV {
                let active = app.page == *page;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
                let p = ui.painter();
                if active {
                    p.rect(rect, theme::radius(10), Color32::from_rgb(24, 30, 22), Stroke::new(1.0, Color32::from_rgb(60, 92, 16)), egui::StrokeKind::Inside);
                    p.circle_filled(egui::pos2(rect.right() - 16.0, rect.center().y), 3.5, GREEN_HI);
                } else if resp.hovered() {
                    p.rect_filled(rect, theme::radius(10), CARD);
                }
                let color = if active { TEXT } else { MUTED };
                p.text(egui::pos2(rect.left() + 16.0, rect.center().y), egui::Align2::LEFT_CENTER, *icon, theme::regular(15.0), color);
                p.text(egui::pos2(rect.left() + 44.0, rect.center().y), egui::Align2::LEFT_CENTER, *label, if active { theme::bold(14.0) } else { theme::regular(14.0) }, color);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    app.page = *page;
                    if *page == Page::History {
                        app.refresh_history();
                    }
                }
                ui.add_space(2.0);
            }
            ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                status_card(app, ui);
                update_banner(app, ui);
            });
        });
}

fn status_card(app: &App, ui: &mut Ui) {
    let scanning = app.games.iter().filter(|g| g.scanning).count();
    let (title, color, progress) = if app.job.is_some() {
        ("Working…", AMBER, None)
    } else if app.discovering {
        ("Finding games…", AMBER, None)
    } else if scanning > 0 {
        ("Scanning…", AMBER, Some(1.0 - scanning as f32 / app.games.len().max(1) as f32))
    } else if app.payload.is_none() {
        ("No payload", RED, Some(0.0))
    } else {
        ("Ready", GREEN_HI, Some(1.0))
    };
    // Laid out bottom-up (the sidebar's bottom_up layout), so in reverse.
    widgets::card_frame().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let frac = progress.unwrap_or_else(|| (ui.input(|i| i.time) as f32 * 0.6).fract());
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
        ui.painter().rect_filled(r, theme::radius(2), BORDER);
        let mut fill = r;
        fill.set_width(r.width() * frac.clamp(0.0, 1.0));
        ui.painter().rect_filled(fill, theme::radius(2), color);
        if scanning > 0 {
            ui.label(RichText::new(format!("{} of {} scanned", app.games.len() - scanning, app.games.len())).size(11.5).color(MUTED));
        }
        match &app.payload {
            Some(p) => ui.label(RichText::new(format!("DLSS {} · ReShade {}", p.dlss_version.clone().unwrap_or_default(), p.reshade_version.clone().unwrap_or_default())).size(11.5).color(MUTED)),
            None => ui.label(RichText::new("Import the DLSS 5 files on Home").size(11.5).color(RED)),
        };
        ui.label(RichText::new(format!("{APP_NAME}  v{}", env!("CARGO_PKG_VERSION"))).size(11.5).color(DIM));
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, color);
            ui.label(RichText::new(title).font(theme::bold(14.0)));
        });
    });
}

/// Shown above the status card while an update is available or running.
fn update_banner(app: &mut App, ui: &mut Ui) {
    let (title, detail, frac, can_click) = match &app.update {
        UpdateState::Available(av) => (
            format!("Update to v{}", av.manifest.version),
            if av.payload_changed { format!("{} MB incl. new DLSS files", (av.manifest.app.size + av.manifest.payload.as_ref().map(|p| p.size).unwrap_or(0)) >> 20) } else { format!("{} MB", av.manifest.app.size >> 20) },
            None,
            true,
        ),
        UpdateState::Applying(f, t) => ("Updating…".to_string(), t.clone(), Some(*f), false),
        _ => return,
    };
    ui.add_space(8.0);
    let resp = egui::Frame::new()
        .fill(Color32::from_rgb(18, 28, 10))
        .stroke(Stroke::new(1.0, GREEN))
        .corner_radius(theme::radius(12))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(f) = frac {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), egui::Sense::hover());
                ui.painter().rect_filled(r, theme::radius(2), BORDER);
                let mut fill = r;
                fill.set_width(r.width() * f.clamp(0.0, 1.0));
                ui.painter().rect_filled(fill, theme::radius(2), GREEN_HI);
            }
            ui.label(RichText::new(detail).size(11.5).color(MUTED));
            ui.label(RichText::new(format!("⬇  {title}")).font(theme::bold(14.0)).color(GREEN_HI));
        })
        .response
        .interact(egui::Sense::click());
    if can_click && resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Download, verify and restart").clicked() {
        app.apply_update();
    }
}

/// Status and setup of the DLSS 5 files: official downloads plus the two
/// core files that only come from a zip or a link the user chooses.
fn import_row(app: &mut App, ui: &mut Ui) {
    use crate::components::{self, CORE, OFFICIAL};
    let root = app.payload.as_ref().map(|p| p.root.clone()).unwrap_or_else(|| app.payload_target());
    let busy = app.importing.clone();
    let missing = components::official_missing(&root);
    ui.add_space(4.0);
    ui.label(RichText::new("1 · Official components").strong());
    if missing.is_empty() {
        let names: Vec<String> = OFFICIAL.iter().filter(|c| !c.label.contains("licence") && !c.label.contains("header")).map(|c| format!("{} {}", c.label, c.version)).collect();
        ui.label(RichText::new(format!("✔ {}", names.join(" · "))).size(12.5).color(GREEN_HI));
    } else {
        let mb: u64 = missing.iter().map(|c| c.size).sum::<u64>() >> 20;
        ui.label(RichText::new(format!("Missing: {}. Downloaded from NVIDIA, reshade.me and GitHub, pinned and SHA-256 verified.", missing.iter().map(|c| c.label).collect::<Vec<_>>().join(", "))).size(12.5).color(MUTED));
        ui.add_enabled_ui(busy.is_none(), |ui| {
            if widgets::accent_button(ui, &format!("⬇  Download official components (~{mb} MB)")).clicked() {
                app.download_official();
            }
        });
    }
    ui.add_space(6.0);
    ui.label(RichText::new("2 · DLSS 5 core files").strong());
    for c in CORE {
        let ok = components::core_present(&root, c);
        let (mark, color) = if ok { ("✔", GREEN_HI) } else if c.required { ("✖", RED) } else { ("–", DIM) };
        ui.label(RichText::new(format!("{mark} {} ({}){}", c.label, c.name, if c.required { "" } else { " · optional" })).size(12.5).color(color));
    }
    ui.label(RichText::new("These have no official download. Use the core zip (or full payload zip) you were given, or a link to it: OneDrive, Google Drive, Dropbox and direct links work.").size(12.0).color(MUTED));
    ui.add_enabled_ui(busy.is_none(), |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.core_link).hint_text("https://… link to NeuralSwap-dlss5-core.zip").desired_width(340.0));
            ui.add(egui::TextEdit::singleline(&mut app.core_sha).hint_text("SHA-256 (optional)").desired_width(170.0));
            if widgets::accent_button(ui, "Download from link").clicked() && !app.core_link.trim().is_empty() {
                app.import_core(None, false);
            }
        });
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, "Import core zip…").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("DLSS 5 core zip", &["zip"]).pick_file() {
                    app.import_core(Some(f), false);
                }
            }
            if widgets::ghost_button(ui, "Import full payload zip…").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("NeuralSwap payload", &["zip"]).pick_file() {
                    app.import_payload(f);
                }
            }
            if widgets::ghost_button(ui, "Import from folder…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    app.import_payload(d);
                }
            }
        });
    });
    if let Some((zip, why)) = app.core_confirm.clone() {
        egui::Frame::new().fill(RED_DARK).stroke(Stroke::new(1.0, RED)).corner_radius(theme::radius(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.label(RichText::new(format!("⚠ {why}.")).color(Color32::from_rgb(255, 190, 186)));
            ui.label(RichText::new("These files are loaded into your games. A modified build from an unknown source can contain malware.").size(12.5).color(Color32::from_rgb(240, 210, 206)));
            ui.horizontal(|ui| {
                if widgets::ghost_button(ui, "Cancel").clicked() {
                    app.core_confirm = None;
                }
                if ui.add(egui::Button::new(RichText::new("Install anyway").color(Color32::WHITE)).fill(Color32::from_rgb(150, 40, 38))).clicked() {
                    app.import_core(Some(zip.clone()), true);
                }
            });
        });
    }
    if let Some((f, t)) = busy {
        ui.label(RichText::new(format!("{t}  ({:.0}%)", f * 100.0)).color(AMBER));
    }
}

fn page_header(ui: &mut Ui, title: &str, subtitle: Option<String>, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(theme::bold(22.0)));
        if let Some(s) = subtitle {
            ui.add_space(4.0);
            ui.label(RichText::new(s).size(13.0).color(DIM));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(10.0);
}

// ---------------------------------------------------------------- home

pub fn home(app: &mut App, ui: &mut Ui) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        widgets::card_frame().inner_margin(egui::Margin::same(26)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("DLSS 5 Neural Rendering, one click away").font(theme::bold(26.0)));
            ui.label(RichText::new("Finds your games, upgrades their DLSS, adds the RenoDX neural-rendering add-on through ReShade — and puts every original file back whenever you want.").size(14.5).color(MUTED));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, "Open my games", true, vec2(180.0, 40.0)).clicked() {
                    app.page = Page::Games;
                }
                if widgets::ghost_button(ui, "↻  Rescan library").clicked() {
                    app.discover();
                }
            });
        });
        ui.add_space(14.0);

        let total = app.games.len();
        let dx12 = app.games.iter().filter(|g| g.target(&app.settings).map(|t| t.api_label.as_deref() == Some("DirectX 12")).unwrap_or(false)).count();
        let ready = app.games.iter().filter(|g| g.ready(&app.settings)).count();
        let with_dlss = app.games.iter().filter(|g| g.dlss_version(&app.settings).is_some()).count();
        let installed = app.games.iter().filter(|g| g.installed_route().is_some()).count();
        let w = (ui.available_width() - 3.0 * 12.0) / 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            widgets::stat_tile(ui, "games found", &total.to_string(), TEXT, w);
            widgets::stat_tile(ui, "ready for DLSS 5", &ready.to_string(), GREEN_HI, w);
            widgets::stat_tile(ui, "ship their own DLSS", &with_dlss.to_string(), BLUE, w);
            widgets::stat_tile(ui, &format!("with DLSS 5 installed · {dx12} on DX12"), &installed.to_string(), AMBER, w);
        });
        ui.add_space(14.0);

        let half = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.vertical(|ui| {
                ui.set_width(half);
                widgets::card_frame().show(ui, |ui| {
                    ui.set_width(half - 34.0);
                    ui.label(RichText::new("Graphics card").font(theme::bold(15.0)));
                    ui.add_space(4.0);
                    match &app.gpu {
                        None => {
                            ui.label(RichText::new("No NVIDIA driver detected (nvidia-smi not found). DLSS requires an NVIDIA RTX GPU.").color(MUTED));
                        }
                        Some(rows) => {
                            for g in rows {
                                widgets::kv_row(ui, &g.name, RichText::new(format!("driver {}", g.driver)).color(TEXT));
                            }
                            if crate::core::guards::driver_neural_fault(rows) {
                                ui.label(RichText::new("⚠ Driver 616.64+ was measured upstream to fault in NGX during neural evaluates with the RenoDX DLSS 5 consumer (DLSS5-Feeder #54). If a game crashes on enabling DLSS 5, 616.56 is the known-good driver.").size(12.5).color(AMBER));
                            }
                            ui.label(RichText::new("RTX 20 / 30 / 40 / 50 supported by the bundled neural runtime.").size(12.0).color(DIM));
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                ui.set_width(half);
                widgets::card_frame().show(ui, |ui| {
                    ui.set_width(half - 34.0);
                    ui.label(RichText::new("Payload").font(theme::bold(15.0)));
                    ui.add_space(4.0);
                    match app.payload.clone().filter(|p| p.complete()) {
                        None => {
                            ui.label(RichText::new("The DLSS 5 files are not set up yet (an original DLSS 5 Swapper install is found automatically).").color(AMBER));
                            import_row(app, ui);
                        }
                        Some(p) => {
                            let v = |o: &Option<String>| o.clone().unwrap_or_else(|| "—".into());
                            widgets::kv_row(ui, "DLSS Super Resolution", RichText::new(v(&p.dlss_version)).color(GREEN_HI));
                            widgets::kv_row(ui, "DLSS 5 Neural Rendering", RichText::new(v(&p.nr_version)).color(GREEN_HI));
                            widgets::kv_row(ui, "ReShade (add-on build)", RichText::new(v(&p.reshade_version)));
                            widgets::kv_row(ui, "RenoDX DLSS 5 consumer", RichText::new(if p.addon.is_some() { "bundled" } else { "missing" }).color(if p.addon.is_some() { TEXT } else { RED }));
                            widgets::kv_row(ui, "RenoDX DLSS Tool (multipass)", RichText::new(if p.multipass_addon.is_some() { "bundled" } else { "—" }));
                            widgets::kv_row(ui, "DLSS5-Feeder", RichText::new(if p.feeder.ok64 { format!("{} verified", crate::core::payload::FEEDER_VERSION) } else { "unavailable".into() }).color(if p.feeder.ok64 { TEXT } else { AMBER }));
                            ui.label(RichText::new(p.root.display().to_string()).size(11.5).color(DIM));
                        }
                    }
                });
            });
        });
        ui.add_space(14.0);
        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Installed").font(theme::bold(15.0)));
            let installed: Vec<(String, String, String)> = app
                .games
                .iter()
                .filter_map(|g| g.installed_route().map(|r| (g.key.clone(), g.lib.name.clone(), r.to_string())))
                .collect();
            if installed.is_empty() {
                ui.label(RichText::new("Nothing installed yet. Open a game and click Install DLSS 5.").color(MUTED));
            }
            for (key, name, route) in installed {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.label(RichText::new(route).size(12.0).color(GREEN_HI));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::ghost_button(ui, "Open").clicked() {
                            app.open_sheet(&key);
                        }
                    });
                });
            }
        });
    });
}

// ---------------------------------------------------------------- games

fn matches(app: &App, idx: usize) -> bool {
    let g = &app.games[idx];
    let f = &app.filters;
    if !f.search.is_empty() && !g.lib.name.to_lowercase().contains(&f.search.to_lowercase()) {
        return false;
    }
    let target = g.target(&app.settings);
    if let Some(api) = &f.api {
        let label = target.as_ref().and_then(|t| t.api_label.clone()).unwrap_or_else(|| "Unknown".into());
        if &label != api {
            return false;
        }
    }
    let dlss = g.dlss_version(&app.settings);
    let ok = match &f.dlss {
        DlssFilter::All => true,
        DlssFilter::Ready => g.ready(&app.settings),
        DlssFilter::Present => dlss.is_some(),
        DlssFilter::None => dlss.is_none(),
        DlssFilter::Installed => g.installed_route().is_some(),
        DlssFilter::Version(v) => dlss.as_deref() == Some(v.as_str()),
    };
    if !ok {
        return false;
    }
    let addon = g.scan.as_ref().map(|s| s.addon_present.is_some()).unwrap_or(false);
    match f.addon {
        AddonFilter::Any => true,
        AddonFilter::Installed => addon,
        AddonFilter::NotInstalled => !addon,
    }
}

fn reason_label(reason: Option<&str>) -> Option<&'static str> {
    match reason? {
        "xbox-protected" => Some("Protected Xbox package"),
        "installer" => Some("Installer, not a game"),
        "no-exe" => Some("No executable"),
        "renderer-in-dll" => Some("Renderer in a separate DLL"),
        _ => Some("No 3D executable"),
    }
}

pub fn games(app: &mut App, ui: &mut Ui) {
    let shown: Vec<usize> = (0..app.games.len()).filter(|&i| matches(app, i)).collect();
    let total = app.games.len();
    let mut add_game = false;
    let mut add_folder = false;
    let mut rescan = false;
    page_header(ui, "Games", Some(format!("{} of {total} shown", shown.len())), |ui| {
        if widgets::accent_button(ui, "↻  Rescan").clicked() {
            rescan = true;
        }
        if widgets::ghost_button(ui, "📁  Add a folder").clicked() {
            add_folder = true;
        }
        if widgets::ghost_button(ui, "➕  Add a game").clicked() {
            add_game = true;
        }
    });
    if rescan {
        app.discover();
    }
    if add_game {
        if let Some(dir) = rfd::FileDialog::new().set_title("Select a game folder").pick_folder() {
            app.add_game_folder(dir);
        }
    }
    if add_folder {
        if let Some(dir) = rfd::FileDialog::new().set_title("Select a folder that contains games").pick_folder() {
            app.add_library_folder(dir);
        }
    }

    // Filters.
    let mut apis: Vec<String> = app.games.iter().filter_map(|g| g.target(&app.settings).and_then(|t| t.api_label)).collect();
    apis.sort();
    apis.dedup();
    let mut versions: Vec<String> = app.games.iter().filter_map(|g| g.dlss_version(&app.settings)).collect();
    versions.sort_by(|a, b| crate::core::pe::compare_versions(Some(b), Some(a)).cmp(&0).then(b.cmp(a)));
    versions.dedup();
    let w = (ui.available_width() - 3.0 * 10.0 - 90.0) / 4.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add_sized(vec2(w * 1.6, 32.0), egui::TextEdit::singleline(&mut app.filters.search).hint_text("🔍  Search by game title…").margin(egui::Margin::symmetric(10, 7)));
        let api_text = app.filters.api.clone().unwrap_or_else(|| "All APIs".into());
        egui::ComboBox::from_id_salt("f-api").width(w * 0.8).height(400.0).selected_text(api_text).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.filters.api, None, "All APIs");
            for a in &apis {
                ui.selectable_value(&mut app.filters.api, Some(a.clone()), a);
            }
        });
        let dlss_text = match &app.filters.dlss {
            DlssFilter::All => "All DLSS states".to_string(),
            DlssFilter::Ready => "Ready to install".into(),
            DlssFilter::Present => "DLSS present".into(),
            DlssFilter::None => "No DLSS".into(),
            DlssFilter::Installed => "DLSS 5 installed".into(),
            DlssFilter::Version(v) => format!("DLSS {v}"),
        };
        egui::ComboBox::from_id_salt("f-dlss").width(w * 0.8).height(400.0).selected_text(dlss_text).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.filters.dlss, DlssFilter::All, "All DLSS states");
            ui.selectable_value(&mut app.filters.dlss, DlssFilter::Ready, "Ready to install");
            ui.selectable_value(&mut app.filters.dlss, DlssFilter::Present, "DLSS present");
            ui.selectable_value(&mut app.filters.dlss, DlssFilter::None, "No DLSS");
            ui.selectable_value(&mut app.filters.dlss, DlssFilter::Installed, "DLSS 5 installed");
            for v in &versions {
                ui.selectable_value(&mut app.filters.dlss, DlssFilter::Version(v.clone()), format!("DLSS {v}"));
            }
        });
        let addon_text = match app.filters.addon {
            AddonFilter::Any => "Any add-on state",
            AddonFilter::Installed => "Add-on installed",
            AddonFilter::NotInstalled => "No add-on",
        };
        egui::ComboBox::from_id_salt("f-addon").width(w * 0.8).selected_text(addon_text).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.filters.addon, AddonFilter::Any, "Any add-on state");
            ui.selectable_value(&mut app.filters.addon, AddonFilter::Installed, "Add-on installed");
            ui.selectable_value(&mut app.filters.addon, AddonFilter::NotInstalled, "No add-on");
        });
        if ui.add(egui::Button::new(RichText::new("Clear").color(DIM)).frame(false)).clicked() {
            app.filters = Default::default();
        }
    });

    let dx12 = app.games.iter().filter(|g| g.target(&app.settings).map(|t| t.api_label.as_deref() == Some("DirectX 12")).unwrap_or(false)).count();
    let ready = app.games.iter().filter(|g| g.ready(&app.settings)).count();
    let with_dlss = app.games.iter().filter(|g| g.dlss_version(&app.settings).is_some()).count();
    let installed = app.games.iter().filter(|g| g.installed_route().is_some()).count();
    ui.horizontal(|ui| {
        if widgets::chip(ui, &format!("{dx12} on DirectX 12"), app.filters.api.as_deref() == Some("DirectX 12")).clicked() {
            app.filters.api = if app.filters.api.as_deref() == Some("DirectX 12") { None } else { Some("DirectX 12".into()) };
        }
        if widgets::chip(ui, &format!("{ready} ready for DLSS 5"), app.filters.dlss == DlssFilter::Ready).clicked() {
            app.filters.dlss = if app.filters.dlss == DlssFilter::Ready { DlssFilter::All } else { DlssFilter::Ready };
        }
        if widgets::chip(ui, &format!("{with_dlss} with DLSS"), app.filters.dlss == DlssFilter::Present).clicked() {
            app.filters.dlss = if app.filters.dlss == DlssFilter::Present { DlssFilter::All } else { DlssFilter::Present };
        }
        if installed > 0 && widgets::chip(ui, &format!("{installed} installed"), app.filters.dlss == DlssFilter::Installed).clicked() {
            app.filters.dlss = if app.filters.dlss == DlssFilter::Installed { DlssFilter::All } else { DlssFilter::Installed };
        }
    });
    ui.add_space(6.0);

    if app.games.is_empty() {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(if app.discovering { "Looking for games…" } else { "No games found yet" }).font(theme::bold(18.0)).color(MUTED));
            ui.label(RichText::new("Steam, Epic, GOG and Ubisoft libraries are found automatically. Drag a game folder onto this window, or use Add a game.").color(DIM));
        });
        return;
    }

    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for &i in &shown {
        let l = app.games[i].lib.launcher.clone();
        match groups.iter_mut().find(|(n, _)| *n == l) {
            Some((_, v)) => v.push(i),
            None => groups.push((l, vec![i])),
        }
    }
    let order = ["Steam", "Epic Games", "GOG", "Ubisoft", "Added"];
    groups.sort_by_key(|(n, _)| order.iter().position(|o| o == n).unwrap_or(99));

    let mut open: Option<String> = None;
    let mut exclude: Option<String> = None;
    let mut rescan_key: Option<String> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (launcher, idxs) in &groups {
            ui.add_space(8.0);
            let ready_here = idxs.iter().filter(|&&i| app.games[i].ready(&app.settings)).count();
            ui.horizontal(|ui| {
                widgets::section_title(ui, launcher, Some(idxs.len()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ready_here > 0 {
                        widgets::chip(ui, &format!("{ready_here} ready for DLSS 5"), true);
                    }
                });
            });
            ui.add_space(4.0);
            let gap = 12.0;
            let avail = ui.available_width();
            let cols = ((avail + gap) / (160.0 + gap)).floor().max(1.0) as usize;
            let card_w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor();
            let card_h = card_w * 1.5 + 50.0;
            for row in idxs.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for &i in row {
                        let visible = ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, vec2(card_w, card_h)));
                        if visible {
                            app.ensure_cover(i);
                        }
                        let g = &app.games[i];
                        let target = g.target(&app.settings);
                        let scan = g.scan.as_ref();
                        let reason = if scan.map(|s| s.exe_candidates.is_empty()).unwrap_or(false) {
                            reason_label(scan.and_then(|s| s.empty_reason.as_deref()))
                        } else {
                            None
                        };
                        let info = CardInfo {
                            name: &g.lib.name,
                            api_label: target.as_ref().and_then(|t| t.api_label.as_deref()),
                            dx12: target.as_ref().map(|t| t.api_label.as_deref() == Some("DirectX 12")).unwrap_or(false),
                            dlss: g.dlss_version(&app.settings),
                            addon: scan.map(|s| s.addon_present.is_some()).unwrap_or(false),
                            installed: g.installed_route().is_some(),
                            disabled_reason: reason,
                            scanning: g.scanning && scan.is_none(),
                        };
                        let resp = widgets::game_card(ui, vec2(card_w, card_h), app.covers.get(&g.key), &info);
                        let key = g.key.clone();
                        let dir = g.lib.dir.clone();
                        if resp.clicked() {
                            open = Some(key.clone());
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Open game sheet").clicked() {
                                open = Some(key.clone());
                                ui.close();
                            }
                            if ui.button("Open folder").clicked() {
                                let _ = open::that(&dir);
                                ui.close();
                            }
                            if ui.button("Rescan").clicked() {
                                rescan_key = Some(key.clone());
                                ui.close();
                            }
                            if ui.button("Remove from library").clicked() {
                                exclude = Some(key.clone());
                                ui.close();
                            }
                        });
                    }
                });
                ui.add_space(gap - 8.0);
            }
        }
        ui.add_space(20.0);
    });
    if let Some(k) = open {
        app.open_sheet(&k);
    }
    if let Some(k) = exclude {
        app.exclude(&k);
    }
    if let Some(k) = rescan_key {
        app.rescan(&k);
    }
}

// ---------------------------------------------------------------- add-ons

pub fn addons(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Add-ons", None, |_| {});
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Bundled").font(theme::bold(15.0)));
            ui.label(RichText::new("Integrated with the installation routes; chosen per game in the game sheet.").size(12.5).color(MUTED));
            ui.add_space(6.0);
            let p = app.payload.as_ref();
            let rows: [(&str, &str, Option<std::path::PathBuf>); 3] = [
                ("RenoDX DLSS 5", "Native DLSS route: hooks the game's own DLSS and runs the neural-rendering pass.", p.and_then(|p| p.addon.clone())),
                ("RenoDX DLSS Tool (multipass)", "ShortFuse's multipass build — its own route, never mixed with the consumer above.", p.and_then(|p| p.multipass_addon.clone())),
                ("DLSS5-Feeder", "Builds a synthetic DLAA contract from ReShade depth and VORT motion vectors for games without DLSS.", p.map(|p| p.feeder.addon64.clone()).filter(|f| f.exists())),
            ];
            for (name, desc, path) in rows {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(desc).size(12.5).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| match path {
                        Some(p) => {
                            ui.label(RichText::new(crate::core::pe::file_version(&p).unwrap_or_else(|| "bundled".into())).color(GREEN_HI));
                        }
                        None => {
                            ui.label(RichText::new("not in payload").color(DIM));
                        }
                    });
                });
            }
        });
        ui.add_space(12.0);
        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Your companion add-ons").font(theme::bold(15.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::accent_button(ui, "➕  Add .addon64").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("ReShade add-on", &["addon64"]).pick_file() {
                            if !app.settings.companions.contains(&f) {
                                app.settings.companions.push(f);
                                app.mark_settings_dirty();
                            }
                        }
                    }
                });
            });
            ui.label(RichText::new("Installed beside the RenoDX consumer on every Native / multipass install (tracked and removed by Restore originals). For advanced users only.").size(12.5).color(MUTED));
            let mut remove = None;
            for (i, c) in app.settings.companions.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(c.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).strong());
                    ui.label(RichText::new(c.display().to_string()).size(11.5).color(if c.exists() { DIM } else { RED }));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::ghost_button(ui, "Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                app.settings.companions.remove(i);
                app.mark_settings_dirty();
            }
            if app.settings.companions.is_empty() {
                ui.label(RichText::new("None.").color(DIM));
            }
        });
    });
}

// ---------------------------------------------------------------- history

pub fn history(app: &mut App, ui: &mut Ui) {
    let mut copy = false;
    page_header(ui, "History", Some(format!("{} entries", app.history.len())), |ui| {
        if widgets::ghost_button(ui, "Copy history").clicked() {
            copy = true;
        }
    });
    if copy {
        let text: Vec<String> = app
            .history
            .iter()
            .map(|r| format!("{}\t{}\t{}\t{}\t{} replaced · {} added\t{}", r.date, r.action, r.name, r.route.clone().unwrap_or_default(), r.replaced, r.added, r.dir))
            .collect();
        ui.ctx().copy_text(text.join("\n"));
        app.toast("History copied to the clipboard.");
    }
    if app.history.is_empty() {
        ui.label(RichText::new("No installs or restores yet.").color(MUTED));
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(theme::radius(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, r) in app.history.iter().enumerate() {
                if i > 0 {
                    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, BORDER));
                }
                egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&r.name).font(theme::bold(14.0)));
                                let (fg, label) = match r.action.as_str() {
                                    "install" => (GREEN_HI, "installed"),
                                    "restore" => (MUTED, "restored"),
                                    _ => (AMBER, "recovered"),
                                };
                                ui.label(RichText::new(label).size(11.5).color(fg));
                                if r.imported {
                                    ui.label(RichText::new("from backup").size(11.0).color(DIM));
                                }
                            });
                            ui.label(RichText::new(&r.dir).size(12.0).color(DIM));
                            if let Some(exe) = &r.exe {
                                ui.label(RichText::new(format!("{exe} · {} · {}", r.route.clone().unwrap_or_else(|| "native".into()), r.api.clone().unwrap_or_default())).size(12.0).color(DIM));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(nice_date(&r.date)).size(12.0).color(MUTED));
                            ui.add_space(12.0);
                            ui.label(RichText::new(format!("{} replaced · {} added", r.replaced, r.added)).size(12.0));
                        });
                    });
                });
            }
        });
    });
}

fn nice_date(iso: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d  %H:%M").to_string())
        .unwrap_or_else(|_| iso.to_string())
}

// ---------------------------------------------------------------- settings

pub fn settings(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Settings", None, |_| {});
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Payload folder").font(theme::bold(15.0)));
            ui.label(RichText::new("The DLSS 5 files: streamline\\ (nvngx_dlss*.dll), renodx-dlss5.addon64, reshade-vulkan\\ and feeder\\. By default a payload folder beside the app (or any parent folder) is used.").size(12.5).color(MUTED));
            ui.add_space(4.0);
            let current = app.payload.as_ref().map(|p| p.root.display().to_string()).unwrap_or_else(|| "not found".into());
            widgets::kv_row(ui, "In use", RichText::new(current).color(if app.payload.is_some() { GREEN_HI } else { RED }));
            widgets::kv_row(ui, "Mode", RichText::new(if app.settings.payload_dir.is_some() { "custom folder" } else { "automatic" }));
            ui.horizontal(|ui| {
                if widgets::accent_button(ui, "Choose folder…").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        app.settings.payload_dir = Some(d);
                        app.mark_settings_dirty();
                        app.reload_payload();
                    }
                }
                if app.settings.payload_dir.is_some() && widgets::ghost_button(ui, "Use automatic").clicked() {
                    app.settings.payload_dir = None;
                    app.mark_settings_dirty();
                    app.reload_payload();
                }
                if let Some(p) = &app.payload {
                    if widgets::ghost_button(ui, "Open").clicked() {
                        let _ = open::that(&p.root);
                    }
                }
            });
            import_row(app, ui);
        });
        ui.add_space(12.0);

        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Library").font(theme::bold(15.0)));
            ui.label(RichText::new("Steam, Epic Games, GOG and Ubisoft Connect libraries are always read. Folders and games you add are scanned too.").size(12.5).color(MUTED));
            ui.add_space(4.0);
            let mut changed = false;
            list_editor(ui, "Library folders", &mut app.settings.folders, &mut changed);
            list_editor(ui, "Added games", &mut app.settings.manual_games, &mut changed);
            list_editor(ui, "Removed from library (click Remove to bring back)", &mut app.settings.excluded, &mut changed);
            if changed {
                app.mark_settings_dirty();
                app.discover();
            }
        });
        ui.add_space(12.0);

        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Updates").font(theme::bold(15.0)));
            ui.label(RichText::new("Updates come straight from the latest GitHub release of a public repository — no server. Every download is checked against the SHA-256 in the release's update.json. The DLSS 5 payload is only downloaded when it changed.").size(12.5).color(MUTED));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("GitHub repository").color(MUTED));
                let mut repo = app.settings.update_repo.clone().unwrap_or_default();
                let hint = crate::update::DEFAULT_REPO.unwrap_or("owner/repo");
                if ui.add(egui::TextEdit::singleline(&mut repo).hint_text(hint).desired_width(260.0)).changed() {
                    app.settings.update_repo = if repo.trim().is_empty() { None } else { Some(repo.trim().to_string()) };
                    app.mark_settings_dirty();
                }
                if widgets::accent_button(ui, "Check now").clicked() {
                    app.check_updates(true);
                }
            });
            if ui.checkbox(&mut app.settings.auto_update_check, "Check for updates when NeuralSwap starts").changed() {
                app.mark_settings_dirty();
            }
            let (text, color) = match &app.update {
                UpdateState::Idle => (format!("Installed: v{}", env!("CARGO_PKG_VERSION")), MUTED),
                UpdateState::Checking => ("Checking GitHub…".to_string(), AMBER),
                UpdateState::UpToDate => (format!("v{} is the latest release.", env!("CARGO_PKG_VERSION")), GREEN_HI),
                UpdateState::Available(av) => (format!("v{} is available.", av.manifest.version), GREEN_HI),
                UpdateState::Applying(f, t) => (format!("{t}  ({:.0}%)", f * 100.0), AMBER),
                UpdateState::Failed(e) => (e.clone(), RED),
            };
            ui.label(RichText::new(text).color(color));
            let mut install = false;
            if let UpdateState::Available(av) = &app.update {
                if !av.manifest.notes.is_empty() {
                    ui.label(RichText::new(&av.manifest.notes).size(12.5));
                }
                ui.horizontal(|ui| {
                    install = widgets::primary_button(ui, "Update and restart", true, vec2(200.0, 36.0)).clicked();
                    if !av.release_page.is_empty() {
                        ui.hyperlink_to("Release notes", &av.release_page);
                    }
                });
            }
            if install {
                app.apply_update();
            }
        });
        ui.add_space(12.0);

        widgets::card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Network & data").font(theme::bold(15.0)));
            if ui.checkbox(&mut app.settings.fetch_art, "Fetch cover art and game details from Steam's public store (no account, nothing sent but the game name)").changed() {
                app.mark_settings_dirty();
            }
            ui.horizontal(|ui| {
                if widgets::ghost_button(ui, "Open data folder").clicked() {
                    let _ = open::that(crate::state::data_dir());
                }
                if widgets::ghost_button(ui, "Clear artwork cache").clicked() {
                    let _ = std::fs::remove_dir_all(crate::art::cache_dir());
                    app.settings.art.clear();
                    app.covers.clear();
                    app.heroes.clear();
                    app.mark_settings_dirty();
                    app.toast("Artwork cache cleared — restart to refetch.");
                }
            });
        });
    });
}

fn list_editor(ui: &mut Ui, title: &str, list: &mut Vec<std::path::PathBuf>, changed: &mut bool) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
    if list.is_empty() {
        ui.label(RichText::new("None").size(12.5).color(DIM));
    }
    let mut remove = None;
    for (i, p) in list.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(p.display().to_string()).size(12.5).color(if p.exists() { TEXT } else { RED }));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        let k = game_key(&list[i]);
        list.retain(|p| game_key(p) != k);
        *changed = true;
    }
}

// ---------------------------------------------------------------- about

pub fn about(app: &mut App, ui: &mut Ui) {
    page_header(ui, "About", None, |_| {});
    widgets::card_frame().inner_margin(egui::Margin::same(22)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(format!("{APP_NAME}  {}", env!("CARGO_PKG_VERSION"))).font(theme::bold(18.0)));
        ui.label(RichText::new("Puts DLSS 5 Neural Rendering into your games, and takes it back out whenever you want. A native Rust rewrite — one small executable, no browser engine.").color(MUTED));
        ui.add_space(10.0);
        ui.label(RichText::new("How it works").strong());
        for line in [
            "1. Finds the game executable from its import and delay-import tables and tells it apart from launchers.",
            "2. Detects the rendering API: static imports, then embedded entry-point names, then the game's own renderer DLLs.",
            "3. Finds the old DLSS runtimes wherever they are buried (including Unreal's ThirdParty folders).",
            "4. Backs every file up and journals every write before touching anything — an interrupted install rolls back.",
            "5. Upgrades DLSS, places the RenoDX add-on and the neural runtime beside the executable, and deploys ReShade.",
            "Restore originals puts every file back and deletes what was added. Backups are compatible with the original DLSS 5 Swapper.",
        ] {
            ui.label(RichText::new(line).size(13.0));
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Credits").strong());
        ui.label(RichText::new("Based on DLSS 5 Swapper by Rakan Alkhaldi (MIT). RenoDX by ShortFuse and contributors. DLSS5-Feeder by jlrouzies-fr. VORT shaders by vortigern11. ReShade by crosire. NVIDIA DLSS / Streamline © NVIDIA Corporation.").size(12.5).color(MUTED));
        ui.horizontal(|ui| {
            ui.hyperlink_to("Original DLSS 5 Swapper", "https://github.com/rakanki911/DLSS5-Swapper");
            ui.hyperlink_to("DLSS5-Feeder", "https://github.com/jlrouzies-fr/DLSS5-Feeder");
            ui.hyperlink_to("ReShade", "https://reshade.me");
        });
        ui.add_space(10.0);
        ui.label(RichText::new("Anti-cheat: nothing here disables or bypasses anti-cheat. Games that use it are flagged, and installing into them needs your explicit consent each time.").size(12.5).color(AMBER));
        let _ = app;
    });
}
