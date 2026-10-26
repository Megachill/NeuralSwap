//! Painted building blocks: game cards, pills, tiles, buttons, toasts.

use crate::i18n::t;
use super::theme::{self, *};
use super::App;
use egui::{pos2, vec2, Align2, Color32, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui};

/// Paints a texture into `rect` with "cover" semantics (crop, never stretch).
pub fn paint_cover(ui: &Ui, rect: Rect, tex: &TextureHandle, radius: u8) {
    let [w, h] = tex.size();
    let (tw, th) = (w as f32, h as f32);
    let target = rect.width() / rect.height();
    let source = tw / th;
    let uv = if source > target {
        let frac = target / source;
        Rect::from_min_max(pos2((1.0 - frac) / 2.0, 0.0), pos2((1.0 + frac) / 2.0, 1.0))
    } else {
        let frac = source / target;
        Rect::from_min_max(pos2(0.0, (1.0 - frac) / 2.0), pos2(1.0, (1.0 + frac) / 2.0))
    };
    egui::Image::from_texture(tex).uv(uv).corner_radius(theme::radius(radius)).paint_at(ui, rect);
}

fn initials(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

pub fn pill(ui: &Ui, pos: egui::Pos2, anchor: Align2, text: &str, fg: Color32, bg: Color32, border: Color32) -> Rect {
    let font = theme::bold(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let size = galley.size() + vec2(14.0, 6.0);
    let rect = anchor.anchor_size(pos, size);
    ui.painter().rect(rect, theme::radius(6), bg, Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().galley(rect.min + vec2(7.0, 3.0), galley, fg);
    rect
}

pub struct CardInfo<'a> {
    pub name: &'a str,
    pub api_label: Option<&'a str>,
    pub dx12: bool,
    pub dlss: Option<String>,
    pub addon: bool,
    pub installed: bool,
    pub disabled_reason: Option<&'a str>,
    pub scanning: bool,
}

pub fn game_card(ui: &mut Ui, size: egui::Vec2, cover: Option<&TextureHandle>, info: &CardInfo) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let hovered = resp.hovered();
    let p = ui.painter();
    let border = if info.installed {
        GREEN
    } else if hovered {
        Color32::from_rgb(90, 104, 118)
    } else {
        BORDER
    };
    p.rect(rect, theme::radius(10), CARD, Stroke::new(if info.installed { 1.5 } else { 1.0 }, border), StrokeKind::Inside);

    let cover_h = (rect.width() * 1.5).min(rect.height() - 52.0);
    let cover_rect = Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(rect.width() - 8.0, cover_h - 4.0));
    match cover {
        Some(tex) => paint_cover(ui, cover_rect, tex, 8),
        None => {
            p.rect_filled(cover_rect, theme::radius(8), Color32::from_rgb(24, 29, 35));
            p.text(cover_rect.center(), Align2::CENTER_CENTER, initials(info.name), theme::bold(30.0), Color32::from_rgb(58, 66, 76));
        }
    }
    if info.disabled_reason.is_some() {
        p.rect_filled(cover_rect, theme::radius(8), Color32::from_black_alpha(140));
    }
    // Bottom shade for the status line.
    let shade = Rect::from_min_max(pos2(cover_rect.left(), cover_rect.bottom() - 34.0), cover_rect.max);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(shade.left_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(shade.right_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(shade.right_bottom(), Color32::from_black_alpha(220));
    mesh.colored_vertex(shade.left_bottom(), Color32::from_black_alpha(220));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);

    // API badge.
    let badge = match (info.disabled_reason, info.api_label) {
        (Some(r), _) => Some((r, MUTED, Color32::from_black_alpha(200), BORDER)),
        (None, Some(l)) if info.dx12 => Some((l, GREEN_HI, Color32::from_rgba_unmultiplied(20, 34, 6, 230), Color32::from_rgb(60, 92, 16))),
        (None, Some(l)) => Some((l, TEXT, Color32::from_black_alpha(200), Color32::from_rgb(60, 66, 74))),
        (None, None) if info.scanning => Some((t("Scanning…"), MUTED, Color32::from_black_alpha(200), BORDER)),
        _ => None,
    };
    if let Some((text, fg, bg, br)) = badge {
        pill(ui, cover_rect.right_top() + vec2(-6.0, 6.0), Align2::RIGHT_TOP, text, fg, bg, br);
    }

    // Status line: DLSS version and add-on state.
    let y = cover_rect.bottom() - 13.0;
    let mut x = cover_rect.left() + 10.0;
    let dot = |x: f32, on: bool| {
        ui.painter().circle_filled(pos2(x, y), 3.5, if on { GREEN_HI } else { Color32::from_rgb(110, 118, 128) });
    };
    let dlss_text = info.dlss.as_deref().map(short_version).unwrap_or_else(|| t("no DLSS").into());
    dot(x, info.dlss.is_some());
    x += 8.0;
    let r = ui.painter().text(pos2(x, y), Align2::LEFT_CENTER, &dlss_text, theme::bold(11.5), TEXT);
    x = r.right() + 12.0;
    dot(x, info.addon);
    ui.painter().text(pos2(x + 8.0, y), Align2::LEFT_CENTER, if info.installed { "DLSS 5" } else { t("add-on") }, theme::bold(11.5), if info.addon { GREEN_HI } else { TEXT });

    // Title.
    let title_rect = Rect::from_min_max(pos2(rect.left() + 10.0, cover_rect.bottom() + 6.0), rect.max - vec2(8.0, 4.0));
    let galley = ui.painter().layout(
        info.name.to_string(),
        theme::bold(13.0),
        if info.disabled_reason.is_some() { MUTED } else { TEXT },
        title_rect.width(),
    );
    let clipped = ui.painter().with_clip_rect(title_rect);
    clipped.galley(title_rect.min, galley, TEXT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "310.8.0.0" → "310.8.0", as on the cards.
pub fn short_version(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() == 4 && parts[3] == "0" { parts[..3].join(".") } else { v.to_string() }
}

pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool, size: egui::Vec2) -> Response {
    let fill = if enabled { GREEN } else { Color32::from_rgb(40, 52, 26) };
    let fg = if enabled { Color32::from_rgb(8, 12, 4) } else { MUTED };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).font(theme::bold(16.0)).color(fg))
            .fill(fill)
            .stroke(Stroke::new(1.0, GREEN_HI))
            .corner_radius(theme::radius(10))
            .min_size(size),
    )
}

pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0))
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(theme::radius(8)),
    )
}

pub fn accent_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0).color(GREEN_HI).strong())
            .fill(Color32::from_rgb(18, 26, 10))
            .stroke(Stroke::new(1.0, Color32::from_rgb(60, 92, 16)))
            .corner_radius(theme::radius(8)),
    )
}

pub fn danger_button(ui: &mut Ui, text: &str, enabled: bool, size: egui::Vec2) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(14.0).color(if enabled { RED } else { DIM }))
            .fill(Color32::from_rgb(26, 16, 18))
            .stroke(Stroke::new(1.0, if enabled { Color32::from_rgb(110, 40, 40) } else { BORDER }))
            .corner_radius(theme::radius(10))
            .min_size(size),
    )
}

pub fn chip(ui: &mut Ui, text: &str, active: bool) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(if active { GREEN_HI } else { MUTED }))
            .fill(if active { Color32::from_rgb(18, 26, 10) } else { PANEL })
            .stroke(Stroke::new(1.0, if active { Color32::from_rgb(60, 92, 16) } else { BORDER }))
            .corner_radius(theme::radius(14)),
    )
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(theme::radius(12))
        .inner_margin(egui::Margin::same(16))
}

pub fn section_title(ui: &mut Ui, title: &str, count: Option<usize>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(theme::bold(15.0)));
        if let Some(c) = count {
            ui.label(RichText::new(c.to_string()).size(11.0).color(DIM));
        }
    });
}

pub fn stat_tile(ui: &mut Ui, label: &str, value: &str, accent: Color32, width: f32) {
    card_frame().show(ui, |ui| {
        ui.set_width(width - 34.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(value).font(theme::bold(28.0)).color(accent));
            ui.label(RichText::new(label).size(12.5).color(MUTED));
        });
    });
}

pub fn kv_row(ui: &mut Ui, key: &str, value: RichText) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).color(MUTED).size(13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value);
        });
    });
}

pub fn toast(app: &mut App, ctx: &egui::Context) {
    let Some((text, at)) = app.toast.clone() else { return };
    let age = at.elapsed().as_secs_f32();
    if age > 4.0 {
        app.toast = None;
        return;
    }
    egui::Area::new(egui::Id::new("toast"))
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -28.0))
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(CARD_HI)
                .stroke(Stroke::new(1.0, GREEN))
                .corner_radius(theme::radius(10))
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new(text).color(TEXT));
                });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
}
