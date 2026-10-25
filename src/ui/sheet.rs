//! The game sheet: what was found, what will change, Install / Restore.

use super::theme::{self, *};
use super::widgets;
use super::{App, API_CHOICES};
use crate::core::apply::Route;
use crate::core::Level;
use crate::state::GamePrefs;
use egui::{vec2, Align, Align2, Color32, Layout, RichText, Stroke, Ui};

enum Action {
    Close,
    Install,
    ConfirmAntiCheat,
    CancelAntiCheat,
    Restore,
    SetExe(String),
    SetApi(String),
    SetRoute(String),
    CopyLog,
    OpenFolder,
    Admin,
    ToggleWarnings,
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_048_576.0)
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(sheet) = app.sheet.as_ref() else { return };
    let key = sheet.key.clone();
    let Some(game) = app.game(&key) else {
        app.sheet = None;
        return;
    };
    let mut actions: Vec<Action> = Vec::new();
    let screen = ctx.content_rect();
    let width = (screen.width() - 80.0).clamp(560.0, 820.0);
    let max_h = screen.height() - 60.0;

    let modal = egui::Modal::new(egui::Id::new("game-sheet"))
        .backdrop_color(Color32::from_black_alpha(170))
        .frame(egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).corner_radius(theme::radius(16)).inner_margin(egui::Margin::ZERO))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_height(max_h);
            egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(max_h).show(ui, |ui| {
                body(app, ui, &key, width, &mut actions);
            });
        });
    if modal.should_close() && !app.sheet.as_ref().map(|s| s.confirm_anticheat).unwrap_or(false) {
        actions.push(Action::Close);
    }
    let _ = game;

    if app.sheet.as_ref().map(|s| s.confirm_anticheat).unwrap_or(false) {
        anti_cheat_dialog(app, ctx, &key, &mut actions);
    }

    for a in actions {
        match a {
            Action::Close => {
                if app.job.is_none() {
                    app.sheet = None;
                }
            }
            Action::Install => {
                let ac = app.game(&key).and_then(|g| g.scan.as_ref()).map(|s| s.anti_cheat).unwrap_or(false);
                if ac {
                    if let Some(s) = app.sheet.as_mut() {
                        s.confirm_anticheat = true;
                    }
                } else {
                    app.start_install(&key, false);
                }
            }
            Action::ConfirmAntiCheat => {
                if let Some(s) = app.sheet.as_mut() {
                    s.confirm_anticheat = false;
                }
                app.start_install(&key, true);
            }
            Action::CancelAntiCheat => {
                if let Some(s) = app.sheet.as_mut() {
                    s.confirm_anticheat = false;
                }
            }
            Action::Restore => app.start_restore(&key),
            Action::SetExe(rel) => {
                let p = app.settings.per_game.entry(key.clone()).or_insert_with(GamePrefs::default);
                p.exe_rel = Some(rel);
                p.route = None;
                app.mark_settings_dirty();
            }
            Action::SetApi(v) => {
                let p = app.settings.per_game.entry(key.clone()).or_insert_with(GamePrefs::default);
                p.api = if v == "auto" { None } else { Some(v) };
                p.route = None;
                app.mark_settings_dirty();
            }
            Action::SetRoute(r) => {
                app.settings.per_game.entry(key.clone()).or_insert_with(GamePrefs::default).route = Some(r);
                app.mark_settings_dirty();
            }
            Action::CopyLog => {
                if let Some(s) = app.sheet.as_ref() {
                    let text: Vec<String> = s.log.iter().map(|e| e.text.clone()).collect();
                    ctx.copy_text(text.join("\n"));
                    app.toast("Log copied.");
                }
            }
            Action::OpenFolder => {
                if let Some(g) = app.game(&key) {
                    let dir = g.target(&app.settings).and_then(|t| t.exe.path.parent().map(|p| p.to_path_buf())).unwrap_or(g.lib.dir.clone());
                    let _ = open::that(dir);
                }
            }
            Action::Admin => app.restart_as_admin(),
            Action::ToggleWarnings => {
                if let Some(s) = app.sheet.as_mut() {
                    s.warnings_open = !s.warnings_open;
                }
            }
        }
    }
}

fn body(app: &App, ui: &mut Ui, key: &str, width: f32, actions: &mut Vec<Action>) {
    let game = app.game(key).unwrap();
    let sheet = app.sheet.as_ref().unwrap();
    let meta = app.settings.art.get(key);
    let scan = game.scan.as_ref();
    let target = game.target(&app.settings);
    let busy = app.job.is_some();
    let busy_here = app.job.as_ref().map(|j| j.key == key).unwrap_or(false);

    // ---- banner
    let banner_h = 190.0;
    let (banner, _) = ui.allocate_exact_size(vec2(width, banner_h), egui::Sense::hover());
    match app.heroes.get(key) {
        Some(h) => widgets::paint_cover(ui, banner, h, 16),
        None => {
            ui.painter().rect_filled(banner, egui::CornerRadius { nw: 16, ne: 16, sw: 0, se: 0 }, Color32::from_rgb(18, 24, 16));
        }
    }
    let mut shade = egui::Mesh::default();
    shade.colored_vertex(banner.left_top(), Color32::from_black_alpha(30));
    shade.colored_vertex(banner.right_top(), Color32::from_black_alpha(30));
    shade.colored_vertex(banner.right_bottom(), PANEL);
    shade.colored_vertex(banner.left_bottom(), PANEL);
    shade.add_triangle(0, 1, 2);
    shade.add_triangle(0, 2, 3);
    ui.painter().add(shade);
    let close = egui::Rect::from_center_size(banner.right_top() + vec2(-28.0, 28.0), vec2(34.0, 34.0));
    let close_resp = ui.interact(close, ui.id().with("close"), egui::Sense::click());
    ui.painter().circle(close.center(), 17.0, Color32::from_black_alpha(if close_resp.hovered() { 230 } else { 160 }), Stroke::new(1.0, BORDER));
    ui.painter().text(close.center(), Align2::CENTER_CENTER, "✖", theme::regular(13.0), TEXT);
    if close_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        actions.push(Action::Close);
    }

    egui::Frame::new().inner_margin(egui::Margin { left: 26, right: 26, top: 0, bottom: 22 }).show(ui, |ui| {
        ui.set_width(width - 52.0);
        // ---- title row with cover thumbnail overlapping the banner
        ui.add_space(-70.0);
        ui.horizontal(|ui| {
            let (thumb, _) = ui.allocate_exact_size(vec2(110.0, 165.0), egui::Sense::hover());
            ui.painter().rect_filled(thumb.expand(2.0), theme::radius(10), BORDER);
            match app.covers.get(key) {
                Some(c) => widgets::paint_cover(ui, thumb, c, 9),
                None => {
                    ui.painter().rect_filled(thumb, theme::radius(9), CARD);
                }
            }
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(70.0);
                let title = meta.and_then(|m| m.title.clone()).unwrap_or_else(|| game.lib.name.clone());
                ui.label(RichText::new(title).font(theme::bold(24.0)));
                let mut sub = vec![game.lib.launcher.clone()];
                if let Some(y) = meta.and_then(|m| m.year) {
                    sub.push(y.to_string());
                }
                if let Some(m) = meta.filter(|m| !m.genres.is_empty()) {
                    sub.push(m.genres.join(", "));
                }
                ui.label(RichText::new(sub.join(" · ")).size(13.0).color(MUTED));
                ui.label(RichText::new(game.lib.dir.display().to_string()).size(12.0).color(DIM));
            });
        });
        if let Some(summary) = meta.and_then(|m| m.summary.clone()) {
            ui.add_space(6.0);
            ui.label(RichText::new(summary).size(13.5).color(Color32::from_rgb(196, 202, 208)));
        }
        ui.add_space(10.0);

        let Some(scan) = scan else {
            ui.label(RichText::new("Scanning…").color(MUTED));
            return;
        };
        let Some(target) = target else {
            let why = match scan.empty_reason.as_deref() {
                Some("xbox-protected") => "This is a protected Microsoft Store / Xbox package. Move or reinstall it to an XboxGames folder through the Xbox app.",
                Some("installer") => "This folder holds an installer, not an installed game.",
                Some("no-exe") => "No executable was found in this folder.",
                Some("renderer-in-dll") => "The renderer lives in a separate DLL that this build cannot target yet.",
                _ => "No executable with a 3D rendering API was found.",
            };
            ui.label(RichText::new(why).color(AMBER));
            if scan.has_backup || scan.pending_recovery {
                ui.add_space(8.0);
                if widgets::danger_button(ui, "Restore originals", !busy, vec2(200.0, 40.0)).clicked() {
                    actions.push(Action::Restore);
                }
            }
            return;
        };
        let routes = game.routes(&app.settings);
        let route = app.chosen_route(key);
        let prefs = app.settings.per_game.get(key);
        let full_w = ui.available_width();

        // ---- executable
        ui.label(RichText::new("Executable").size(12.0).color(MUTED));
        let exe_text = |c: &crate::core::scan::ExeCandidate| {
            format!(
                "{} — {} — {}-bit — {}",
                c.rel,
                c.api_label.clone().unwrap_or_else(|| "API unknown".into()),
                c.bitness,
                mb(c.size)
            )
        };
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("sheet-exe").width(full_w).selected_text(exe_text(&target.exe)).show_ui(ui, |ui| {
                for c in &scan.exe_candidates {
                    if ui.selectable_label(c.rel == target.exe.rel, exe_text(c)).clicked() {
                        actions.push(Action::SetExe(c.rel.clone()));
                    }
                }
            });
        });
        ui.add_space(6.0);

        // ---- API + route
        let half = (full_w - 12.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.vertical(|ui| {
                ui.set_width(half);
                ui.label(RichText::new("Rendering API").size(12.0).color(MUTED));
                let current = prefs.and_then(|p| p.api.clone()).unwrap_or_else(|| "auto".into());
                let auto_text = format!("Automatic (detected: {})", target.exe.api_label.clone().unwrap_or_else(|| "unknown".into()));
                let text = API_CHOICES.iter().find(|c| c.0 == current).map(|c| c.2.to_string()).unwrap_or(auto_text.clone());
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_id_salt("sheet-api").width(half).selected_text(text).show_ui(ui, |ui| {
                        if ui.selectable_label(current == "auto", auto_text).clicked() {
                            actions.push(Action::SetApi("auto".into()));
                        }
                        for (v, _, label) in API_CHOICES {
                            if ui.selectable_label(current == *v, *label).clicked() {
                                actions.push(Action::SetApi(v.to_string()));
                            }
                        }
                    });
                });
            });
            ui.vertical(|ui| {
                ui.set_width(half);
                ui.label(RichText::new("Installation route").size(12.0).color(MUTED));
                let text = route.map(|r| r.label().to_string()).unwrap_or_else(|| "No supported route".into());
                ui.add_enabled_ui(!busy && !routes.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt("sheet-route").width(half).selected_text(text).show_ui(ui, |ui| {
                        for r in &routes {
                            let available = match r {
                                Route::Renodx => app.payload.as_ref().map(|p| p.multipass_addon.is_some()).unwrap_or(false),
                                Route::Feeder => app.payload.as_ref().map(|p| p.feeder.ok64).unwrap_or(false),
                                Route::Native => true,
                            };
                            let resp = ui.add_enabled(available, egui::Button::selectable(Some(*r) == route, r.label()));
                            if resp.clicked() {
                                actions.push(Action::SetRoute(r.key().into()));
                            }
                        }
                    });
                });
            });
        });
        ui.add_space(10.0);

        // ---- warnings
        let mut notes: Vec<(String, bool)> = Vec::new();
        match route {
            Some(Route::Native) => notes.push(("Native DLSS upgrades the game's own DLSS runtime and adds the RenoDX DLSS 5 add-on through ReShade. Streamline and frame-generation files are preserved. Enable DLSS in the game's settings, then check the add-on in ReShade's Add-ons tab (Home key). \"No effects found\" alone does not mean installation failed.".into(), false)),
            Some(Route::Feeder) => notes.push(("DLSS5-Feeder builds a synthetic DLAA contract from ReShade depth and VORT motion vectors, so the game does not need DLSS of its own. Experimental: select the correct depth buffer in ReShade if the output looks wrong.".into(), false)),
            Some(Route::Renodx) => notes.push(("RenoDX DLSS Tool (multipass) hooks Present and runs its own passes. It replaces, never joins, the ordinary RenoDX DLSS 5 consumer.".into(), false)),
            None => notes.push((format!(
                "No installation route for {} {}-bit in this build. Native / multipass / Feeder routes cover 64-bit DirectX 11 and 12 games.",
                target.api_label.clone().unwrap_or_else(|| "this API".into()),
                target.exe.bitness
            ), false)),
        }
        if target.exe.via == "undetected" {
            notes.push(("The rendering API of this executable could not be detected — pick it under Rendering API if you know it.".into(), false));
        }
        if let Some(gpus) = &app.gpu {
            if crate::core::guards::driver_neural_fault(gpus) {
                notes.push(("Your NVIDIA driver is 616.64 or newer, which was measured upstream to fault inside NGX with the RenoDX DLSS 5 consumer. If the game crashes when DLSS 5 engages, try driver 616.56.".into(), false));
            }
        }
        if scan.pending_recovery {
            notes.push(("An interrupted install was found. Click Restore originals to roll it back before installing again.".into(), true));
        }
        if scan.anti_cheat {
            notes.push(("Warning: anti-cheat detected\nThis game appears to use anti-cheat (EasyAntiCheat / BattlEye). ReShade, Feeder or a DLSS add-on may cause crashes, prevent launch, or result in an account ban. Installation is optional: click Install and explicitly accept the risk to continue. Anti-cheat is never disabled or bypassed.".into(), true));
        }
        let severe = notes.iter().filter(|n| n.1).count();
        let (border, fill, head) = if severe > 0 { (Color32::from_rgb(120, 40, 42), Color32::from_rgb(30, 15, 17), RED) } else { (Color32::from_rgb(52, 76, 20), Color32::from_rgb(17, 23, 12), GREEN_HI) };
        egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, border)).corner_radius(theme::radius(12)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let resp = ui.horizontal(|ui| {
                ui.label(RichText::new(if severe > 0 { "⚠  Warning — read before installing" } else { "ℹ  Before installing" }).font(theme::bold(14.0)).color(head));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(if sheet.warnings_open { "⏶" } else { "⏷" }).color(MUTED));
                    ui.label(RichText::new(notes.len().to_string()).size(11.5).color(head));
                });
            });
            if resp.response.interact(egui::Sense::click()).clicked() {
                actions.push(Action::ToggleWarnings);
            }
            if sheet.warnings_open {
                for (text, bad) in &notes {
                    ui.add_space(4.0);
                    let (f, s, c) = if *bad { (RED_DARK, Color32::from_rgb(150, 50, 50), Color32::from_rgb(255, 190, 186)) } else { (Color32::from_rgb(24, 32, 16), Color32::from_rgb(56, 76, 30), Color32::from_rgb(170, 184, 150)) };
                    egui::Frame::new().fill(f).stroke(Stroke::new(1.0, s)).corner_radius(theme::radius(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut lines = text.splitn(2, '\n');
                        let first = lines.next().unwrap_or("");
                        match lines.next() {
                            Some(rest) => {
                                ui.label(RichText::new(first).strong().color(if *bad { Color32::from_rgb(255, 150, 140) } else { c }));
                                ui.label(RichText::new(rest).size(12.5).color(c));
                            }
                            None => {
                                ui.label(RichText::new(first).size(12.5).color(c));
                            }
                        }
                    });
                }
            }
        });
        ui.add_space(10.0);

        // ---- facts
        let primary = scan.primary_dlss(Some(&target.exe));
        let payload = app.payload.as_ref();
        egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(theme::radius(12)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let row = |ui: &mut Ui, k: &str, v: RichText| {
                widgets::kv_row(ui, k, v);
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top() - 3.0, Stroke::new(1.0, Color32::from_rgb(30, 36, 42)));
            };
            row(ui, "Architecture", RichText::new(format!("{}-bit", target.exe.bitness)).strong());
            let api_color = if target.api_label.as_deref() == Some("DirectX 12") { GREEN_HI } else { TEXT };
            row(ui, "Rendering API", RichText::new(target.api_label.clone().unwrap_or_else(|| "unknown".into())).strong().color(api_color));
            row(ui, "Installed route", RichText::new(game.installed_route().map(|r| Route::from_key(r).map(|r| r.label()).unwrap_or(r)).unwrap_or("none")).color(if game.installed_route().is_some() { GREEN_HI } else { DIM }));
            let new = payload.and_then(|p| p.dlss_version.clone()).unwrap_or_default();
            let dlss_text = match primary.and_then(|p| p.version.clone()) {
                Some(v) if v == new => RichText::new(format!("{v}  ✔")).color(GREEN_HI),
                Some(v) => RichText::new(format!("{v}  →  {new}")).color(GREEN_HI),
                None => RichText::new(if route == Some(Route::Native) { format!("none  →  {new}") } else { "none (Feeder supplies it)".into() }).color(MUTED),
            };
            row(ui, "DLSS", dlss_text);
            row(ui, "DLSS 5 add-on", RichText::new(scan.addon_present.clone().unwrap_or_else(|| "Not present".into())).color(if scan.addon_present.is_some() { GREEN_HI } else { DIM }));
            let rs = &scan.reshade;
            let rs_text = if rs.installed {
                format!("{} {} ({}{})", rs.file.clone().unwrap_or_default(), rs.version.clone().unwrap_or_default(), rs.kind.clone().unwrap_or_default(), if rs.addon_support { ", add-on build" } else { "" })
            } else {
                "Not installed".into()
            };
            widgets::kv_row(ui, "ReShade", RichText::new(rs_text).color(if rs.installed { TEXT } else { DIM }));
        });
        ui.add_space(8.0);

        // ---- runtime files
        let files: Vec<_> = scan.dlss_files.iter().chain(scan.streamline_files.iter()).collect();
        if !files.is_empty() {
            egui::Frame::new().fill(FIELD).stroke(Stroke::new(1.0, BORDER)).corner_radius(theme::radius(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("sheet-files").max_height(120.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for f in files {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&f.rel).monospace().size(12.0));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(f.version.clone().unwrap_or_default()).monospace().size(12.0).color(MUTED));
                            });
                        });
                    }
                });
            });
            ui.add_space(10.0);
        }

        // ---- buttons
        let can_install = !busy && route.is_some() && payload.is_some() && !scan.pending_recovery;
        let has_backup = scan.has_backup || scan.pending_recovery;
        ui.horizontal(|ui| {
            let installed = game.installed_route().is_some();
            let label = if busy_here && matches!(app.job.as_ref().map(|j| &j.kind), Some(super::JobKind::Install)) {
                "Installing…".to_string()
            } else if installed {
                "Reinstall DLSS 5".to_string()
            } else {
                "Install DLSS 5".to_string()
            };
            let w = ui.available_width() - 200.0 - 12.0;
            if widgets::primary_button(ui, &label, can_install, vec2(w, 48.0)).clicked() {
                actions.push(Action::Install);
            }
            if widgets::danger_button(ui, if busy_here && matches!(app.job.as_ref().map(|j| &j.kind), Some(super::JobKind::Restore)) { "Restoring…" } else { "Restore originals" }, !busy && has_backup, vec2(200.0, 48.0)).clicked() {
                actions.push(Action::Restore);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, "Open game folder").clicked() {
                actions.push(Action::OpenFolder);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::ghost_button(ui, "Copy log").clicked() {
                    actions.push(Action::CopyLog);
                }
                if sheet.last_error == Some("errNoWriteAccess") && widgets::accent_button(ui, "Restart as administrator").clicked() {
                    actions.push(Action::Admin);
                }
            });
        });
        ui.add_space(6.0);

        // ---- log
        egui::Frame::new().fill(FIELD).stroke(Stroke::new(1.0, BORDER)).corner_radius(theme::radius(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("sheet-log").max_height(200.0).stick_to_bottom(true).show(ui, |ui| {
                if sheet.log.is_empty() {
                    let ready = if payload.is_none() { "No payload — set it in Settings." } else if route.is_none() { "Nothing to install for this executable." } else { "Ready." };
                    ui.label(RichText::new(ready).monospace().size(12.0).color(MUTED));
                }
                for e in &sheet.log {
                    let (tag, color) = match e.level {
                        Level::Info => ("  ", MUTED),
                        Level::Ok => ("✔ ", GREEN_HI),
                        Level::Warn => ("! ", AMBER),
                        Level::Error => ("✖ ", RED),
                    };
                    ui.label(RichText::new(format!("{tag}{}", e.text)).monospace().size(12.0).color(color));
                }
            });
        });
    });
}

fn anti_cheat_dialog(app: &App, ctx: &egui::Context, key: &str, actions: &mut Vec<Action>) {
    let game = app.game(key);
    let exe = game.and_then(|g| g.target(&app.settings)).map(|t| t.exe.path.display().to_string()).unwrap_or_default();
    let dir = game.map(|g| g.lib.dir.display().to_string()).unwrap_or_default();
    egui::Modal::new(egui::Id::new("anticheat"))
        .backdrop_color(Color32::from_black_alpha(120))
        .frame(egui::Frame::new().fill(Color32::from_rgb(24, 14, 16)).stroke(Stroke::new(1.5, RED)).corner_radius(theme::radius(14)).inner_margin(egui::Margin::same(22)))
        .show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new("⚠  Warning: anti-cheat detected").font(theme::bold(19.0)).color(Color32::from_rgb(255, 140, 130)));
            ui.add_space(6.0);
            ui.label(RichText::new("This game appears to use anti-cheat. Injecting ReShade and the DLSS 5 add-on may cause crashes, prevent the game from launching, or result in an account ban. Anti-cheat will not be disabled or bypassed — this is entirely your decision, and it applies to this one installation only.").color(Color32::from_rgb(240, 210, 206)));
            ui.add_space(8.0);
            ui.label(RichText::new(dir).monospace().size(11.5).color(MUTED));
            ui.label(RichText::new(exe).monospace().size(11.5).color(MUTED));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if widgets::ghost_button(ui, "Cancel").clicked() {
                    actions.push(Action::CancelAntiCheat);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("I accept the risk — install").strong().color(Color32::WHITE))
                                .fill(Color32::from_rgb(150, 40, 38))
                                .stroke(Stroke::new(1.0, RED))
                                .corner_radius(theme::radius(8)),
                        )
                        .clicked()
                    {
                        actions.push(Action::ConfirmAntiCheat);
                    }
                });
            });
        });
}
