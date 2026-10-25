//! Dark "RTX green" look: near-black panels, NVIDIA-style accent, Segoe UI.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(9, 11, 14);
pub const PANEL: Color32 = Color32::from_rgb(14, 17, 21);
pub const CARD: Color32 = Color32::from_rgb(20, 24, 29);
pub const CARD_HI: Color32 = Color32::from_rgb(27, 32, 38);
pub const FIELD: Color32 = Color32::from_rgb(17, 21, 26);
pub const BORDER: Color32 = Color32::from_rgb(38, 45, 53);
pub const TEXT: Color32 = Color32::from_rgb(232, 236, 240);
pub const MUTED: Color32 = Color32::from_rgb(139, 148, 158);
pub const DIM: Color32 = Color32::from_rgb(96, 104, 114);
pub const GREEN: Color32 = Color32::from_rgb(118, 185, 0);
pub const GREEN_HI: Color32 = Color32::from_rgb(143, 212, 0);
pub const GREEN_DARK: Color32 = Color32::from_rgb(34, 52, 10);
pub const RED: Color32 = Color32::from_rgb(229, 83, 75);
pub const RED_DARK: Color32 = Color32::from_rgb(58, 22, 24);
pub const AMBER: Color32 = Color32::from_rgb(230, 170, 50);
pub const BLUE: Color32 = Color32::from_rgb(88, 166, 255);

pub fn radius(r: u8) -> CornerRadius {
    CornerRadius::same(r)
}

fn load_font(path: &str) -> Option<FontData> {
    std::fs::read(path).ok().map(FontData::from_owned)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let fonts_dir = format!("{windir}\\Fonts");
    if let Some(regular) = load_font(&format!("{fonts_dir}\\segoeui.ttf")) {
        fonts.font_data.insert("segoe".into(), Arc::new(regular));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    let bold = load_font(&format!("{fonts_dir}\\seguisb.ttf")).or_else(|| load_font(&format!("{fonts_dir}\\segoeuib.ttf")));
    let mut bold_family = Vec::new();
    if let Some(b) = bold {
        fonts.font_data.insert("segoe-bold".into(), Arc::new(b));
        bold_family.push("segoe-bold".to_string());
    }
    // Keep egui's own fonts as fallbacks for the icon glyphs.
    bold_family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name("bold".into()), bold_family);
    if let Some(mono) = load_font(&format!("{fonts_dir}\\consola.ttf")) {
        fonts.font_data.insert("consolas".into(), Arc::new(mono));
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "consolas".into());
    }
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = CARD;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = radius(14);
    v.menu_corner_radius = radius(8);
    v.selection.bg_fill = GREEN_DARK;
    v.selection.stroke = Stroke::new(1.0, GREEN_HI);
    v.hyperlink_color = GREEN_HI;
    v.override_text_color = Some(TEXT);
    let w = &mut v.widgets;
    for (s, fill, stroke) in [
        (&mut w.noninteractive, CARD, BORDER),
        (&mut w.inactive, FIELD, BORDER),
        (&mut w.hovered, CARD_HI, Color32::from_rgb(70, 82, 94)),
        (&mut w.active, CARD_HI, GREEN),
        (&mut w.open, CARD_HI, GREEN),
    ] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.bg_stroke = Stroke::new(1.0, stroke);
        s.corner_radius = radius(8);
    }
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.set_theme(egui::ThemePreference::Dark);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.combo_height = 320.0;
        style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        style.text_styles.insert(TextStyle::Heading, FontId::new(22.0, FontFamily::Name("bold".into())));
        style.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.5));
    });
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
