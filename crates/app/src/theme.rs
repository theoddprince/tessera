//! Colours, fonts and egui style.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub surface_hi: Color32,
    pub border: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub info: Color32,
    pub json_key: Color32,
    pub json_str: Color32,
    pub json_num: Color32,
    pub json_kw: Color32,
    pub punct: Color32,
}

/// Tessera brand colours (from the logo).
pub const BRAND_BLUE: Color32 = Color32::from_rgb(0x4c, 0x7d, 0xff);
pub const BRAND_BLUE_DEEP: Color32 = Color32::from_rgb(0x2f, 0x57, 0xd6);
pub const BRAND_ORANGE: Color32 = Color32::from_rgb(0xf2, 0xa3, 0x3a);

pub const DARK: Palette = Palette {
    // Matches the wordmark background so the logo sits seamlessly in the top bar.
    bg: Color32::from_rgb(0x0b, 0x0d, 0x14),
    panel: Color32::from_rgb(0x12, 0x15, 0x21),
    surface: Color32::from_rgb(0x18, 0x1c, 0x29),
    surface_hi: Color32::from_rgb(0x23, 0x28, 0x38),
    border: Color32::from_rgb(0x29, 0x2f, 0x41),
    text: Color32::from_rgb(0xdd, 0xe1, 0xe8),
    dim: Color32::from_rgb(0x8a, 0x93, 0xa5),
    accent: BRAND_BLUE,
    ok: Color32::from_rgb(0x3d, 0xd6, 0x8c),
    warn: BRAND_ORANGE,
    err: Color32::from_rgb(0xff, 0x6b, 0x6b),
    info: Color32::from_rgb(0x5a, 0xa9, 0xff),
    json_key: Color32::from_rgb(0x9c, 0xdc, 0xfe),
    json_str: Color32::from_rgb(0xce, 0x91, 0x78),
    json_num: Color32::from_rgb(0xb5, 0xce, 0xa8),
    json_kw: Color32::from_rgb(0x56, 0x9c, 0xd6),
    punct: Color32::from_rgb(0x9a, 0xa2, 0xb1),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xf3, 0xf4, 0xf7),
    panel: Color32::from_rgb(0xfa, 0xfb, 0xfc),
    surface: Color32::from_rgb(0xff, 0xff, 0xff),
    surface_hi: Color32::from_rgb(0xea, 0xec, 0xf1),
    border: Color32::from_rgb(0xdc, 0xdf, 0xe6),
    text: Color32::from_rgb(0x1f, 0x23, 0x2b),
    dim: Color32::from_rgb(0x6a, 0x72, 0x82),
    accent: BRAND_BLUE_DEEP,
    ok: Color32::from_rgb(0x1a, 0x9d, 0x5e),
    warn: Color32::from_rgb(0xc7, 0x7c, 0x00),
    err: Color32::from_rgb(0xd6, 0x3a, 0x3a),
    info: Color32::from_rgb(0x1f, 0x6f, 0xd1),
    json_key: Color32::from_rgb(0x04, 0x51, 0xa5),
    json_str: Color32::from_rgb(0xa3, 0x15, 0x15),
    json_num: Color32::from_rgb(0x09, 0x86, 0x58),
    json_kw: Color32::from_rgb(0x00, 0x00, 0xff),
    punct: Color32::from_rgb(0x55, 0x5d, 0x6b),
};

pub fn palette(dark: bool) -> Palette {
    if dark {
        DARK
    } else {
        LIGHT
    }
}

pub fn method_color(method: &str, dark: bool) -> Color32 {
    let p = palette(dark);
    match method {
        "GET" => p.ok,
        "POST" => p.warn,
        "PUT" => p.info,
        "PATCH" => Color32::from_rgb(0xb0, 0x7c, 0xff),
        "DELETE" => p.err,
        "GQL" => Color32::from_rgb(0xe5, 0x35, 0xab),
        "WS" | "OPTIONS" => Color32::from_rgb(0x2e, 0xc4, 0xd6),
        _ => p.dim,
    }
}

pub fn status_color(status: u16, dark: bool) -> Color32 {
    let p = palette(dark);
    match status {
        200..=299 => p.ok,
        300..=399 => p.info,
        400..=499 => p.warn,
        _ => p.err,
    }
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.surface;
    visuals.faint_bg_color = p.surface;
    visuals.code_bg_color = p.surface;
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    visuals.window_stroke = Stroke::new(1.0_f32, p.border);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if dark { 120 } else { 40 }) };
    visuals.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(if dark { 100 } else { 30 }) };
    visuals.striped = true;
    let r = CornerRadius::same(6);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.surface_hi;
    w.hovered.weak_bg_fill = p.surface_hi;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.hovered.corner_radius = r;
    w.active.bg_fill = p.surface_hi;
    w.active.weak_bg_fill = p.surface_hi;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.active.corner_radius = r;
    w.open.bg_fill = p.surface_hi;
    w.open.weak_bg_fill = p.surface_hi;
    w.open.corner_radius = r;
    ctx.set_visuals(visuals);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.scroll = egui::style::ScrollStyle::thin();
        s.text_styles = [
            (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();
    });
}

/// Use Windows' own UI fonts when available (crisper, native look); egui's bundled fonts stay as fallbacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR").map(|w| std::path::PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let mut add = |name: &str, file: &str, family: FontFamily, front: bool| {
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
            let list = fonts.families.entry(family).or_default();
            if front {
                list.insert(0, name.to_owned());
            } else {
                list.push(name.to_owned());
            }
        }
    };
    add("segoe", "segoeui.ttf", FontFamily::Proportional, true);
    add("cascadia", "CascadiaMono.ttf", FontFamily::Monospace, true);
    add("symbols", "seguisym.ttf", FontFamily::Proportional, false);
    add("symbols-mono", "seguisym.ttf", FontFamily::Monospace, false);
    ctx.set_fonts(fonts);
}

pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");
pub const WORDMARK_DARK_PNG: &[u8] = include_bytes!("../assets/wordmark-dark.png");
pub const WORDMARK_LIGHT_PNG: &[u8] = include_bytes!("../assets/wordmark-light.png");

/// Background the wordmark images were drawn on, per theme.
pub fn wordmark_bg(dark: bool) -> Color32 {
    if dark {
        DARK.bg
    } else {
        Color32::WHITE
    }
}

/// Window / taskbar icon decoded from the bundled PNG.
pub fn app_icon() -> egui::IconData {
    match image::load_from_memory(ICON_PNG) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData { rgba: rgba.into_raw(), width, height }
        }
        Err(_) => egui::IconData::default(),
    }
}
