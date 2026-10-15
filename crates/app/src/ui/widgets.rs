//! Reusable widgets: tab strips, badges, key/value tables, highlighted editors.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, RichText, Sense, Ui, Vec2};
use tessera_core::model::{KeyValue, KvKind};
use tessera_core::vars::{find_vars, VarScope};

use crate::theme::{self, Palette};

pub fn pal(ui: &Ui) -> Palette {
    theme::palette(ui.visuals().dark_mode)
}

/// Underlined text tabs (Params | Headers | ...).
pub fn tab_strip<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, String)]) -> bool {
    let p = pal(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (value, label) in items {
            let selected = *current == *value;
            let text = RichText::new(label).color(if selected { p.text } else { p.dim });
            let resp = ui.add(egui::Label::new(text).sense(Sense::click()));
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if selected {
                let r = resp.rect;
                ui.painter().line_segment(
                    [egui::pos2(r.left(), r.bottom() + 3.0), egui::pos2(r.right(), r.bottom() + 3.0)],
                    egui::Stroke::new(2.0_f32, p.accent),
                );
            } else if resp.hovered() {
                let r = resp.rect;
                ui.painter().line_segment(
                    [egui::pos2(r.left(), r.bottom() + 3.0), egui::pos2(r.right(), r.bottom() + 3.0)],
                    egui::Stroke::new(1.0_f32, p.border),
                );
            }
            if resp.clicked() && !selected {
                *current = *value;
                changed = true;
            }
        }
    });
    changed
}

/// Segmented control (Pretty | Raw | Tree).
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, &str)]) -> bool {
    let p = pal(ui);
    let mut changed = false;
    egui::Frame::new()
        .fill(p.surface)
        .corner_radius(6)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (value, label) in items {
                    let selected = *current == *value;
                    let text = RichText::new(*label).size(12.0).color(if selected { p.text } else { p.dim });
                    let btn = egui::Button::new(text)
                        .fill(if selected { p.surface_hi } else { Color32::TRANSPARENT })
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(4)
                        .min_size(egui::vec2(0.0, 20.0));
                    if ui.add(btn).clicked() && !selected {
                        *current = *value;
                        changed = true;
                    }
                }
            });
        });
    changed
}

pub fn method_text(method: &str, dark: bool, size: f32) -> RichText {
    let short = match method {
        "DELETE" => "DEL",
        "OPTIONS" => "OPT",
        "PATCH" => "PATCH",
        m => m,
    };
    RichText::new(short).color(theme::method_color(method, dark)).strong().size(size).family(egui::FontFamily::Monospace)
}

/// Fixed-width method label for lists.
pub fn method_badge(ui: &mut Ui, method: &str) {
    let dark = ui.visuals().dark_mode;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(38.0, 16.0), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        match method {
            "DELETE" => "DEL",
            "OPTIONS" => "OPT",
            "PATCH" => "PTCH",
            m => m,
        },
        FontId::monospace(10.5),
        theme::method_color(method, dark),
    );
}

pub fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.15))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| ui.label(RichText::new(text.into()).color(color).strong().size(12.0)))
        .response
}

pub fn dim(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(pal(ui).dim)
}

pub fn human_size(bytes: usize) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{:.2} MB", b / 1024.0 / 1024.0)
    }
}

pub fn human_time(ms: f64) -> String {
    if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

// ------------------------------------------------------------------ highlighting

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Plain,
    Json,
    JavaScript,
    GraphQL,
    Markup,
}

const JS_KEYWORDS: &[&str] = &[
    "var", "let", "const", "function", "return", "if", "else", "for", "while", "do", "break", "continue", "new",
    "typeof", "instanceof", "in", "of", "try", "catch", "finally", "throw", "switch", "case", "default", "this",
    "class", "extends", "async", "await", "delete", "void",
];
const GQL_KEYWORDS: &[&str] =
    &["query", "mutation", "subscription", "fragment", "on", "type", "input", "enum", "interface", "schema", "scalar", "union"];

fn fmt(font: &FontId, color: Color32) -> TextFormat {
    TextFormat { font_id: font.clone(), color, ..Default::default() }
}

/// Tokenising highlighter for JSON / JS / GraphQL / markup, with `{{var}}` awareness.
pub fn highlight(text: &str, lang: Lang, p: &Palette, font: &FontId, scope: Option<&VarScope>) -> LayoutJob {
    let mut job = LayoutJob::default();
    if lang == Lang::Plain && scope.is_none() || text.len() > 300_000 {
        job.append(text, 0.0, fmt(font, p.text));
        return job;
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut plain_start = 0;
    let flush = |job: &mut LayoutJob, from: usize, to: usize, color: Color32| {
        if to > from {
            job.append(&text[from..to], 0.0, fmt(font, color));
        }
    };
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        let mut color = None;
        if c == b'{' && bytes.get(i + 1) == Some(&b'{') {
            if let Some(end) = text[i + 2..].find("}}") {
                let name = text[i + 2..i + 2 + end].trim();
                if !name.is_empty() && !name.contains('\n') {
                    i = i + 2 + end + 2;
                    color = Some(match scope {
                        Some(s) if s.lookup(name).is_some() => p.ok,
                        Some(_) => p.err,
                        None => p.accent,
                    });
                }
            }
        }
        if color.is_none() && lang != Lang::Plain {
            match c {
                b'"' | b'\'' | b'`' if lang != Lang::Markup || c == b'"' => {
                    if lang == Lang::Json && c != b'"' {
                        i += 1;
                        continue;
                    }
                    i += 1;
                    while i < bytes.len() && bytes[i] != c && bytes[i] != b'\n' {
                        if bytes[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i = (i + 1).min(bytes.len());
                    let mut j = i;
                    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                        j += 1;
                    }
                    let is_key = lang == Lang::Json && bytes.get(j) == Some(&b':');
                    color = Some(if is_key { p.json_key } else { p.json_str });
                }
                b'/' if lang == Lang::JavaScript && bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    color = Some(p.dim);
                }
                b'/' if lang == Lang::JavaScript && bytes.get(i + 1) == Some(&b'*') => {
                    i = text[i + 2..].find("*/").map(|e| i + 2 + e + 2).unwrap_or(bytes.len());
                    color = Some(p.dim);
                }
                b'#' if lang == Lang::GraphQL => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    color = Some(p.dim);
                }
                b'<' if lang == Lang::Markup => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'>' && bytes[i] != b' ' && bytes[i] != b'\n' {
                        i += 1;
                    }
                    if bytes.get(i) == Some(&b'>') {
                        i += 1;
                    }
                    color = Some(p.json_kw);
                }
                b'>' if lang == Lang::Markup => {
                    i += 1;
                    color = Some(p.json_kw);
                }
                b'0'..=b'9' | b'-' if lang != Lang::Markup => {
                    let prev_ident = start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
                    if !prev_ident && (c != b'-' || bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                        i += 1;
                        while i < bytes.len() && (bytes[i].is_ascii_digit() || matches!(bytes[i], b'.' | b'e' | b'E' | b'+' | b'-')) {
                            i += 1;
                        }
                        color = Some(p.json_num);
                    }
                }
                b'$' | b'_' | b'a'..=b'z' | b'A'..=b'Z' if lang != Lang::Markup => {
                    let prev_ident = start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
                    if !prev_ident {
                        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$') {
                            i += 1;
                        }
                        let word = &text[start..i];
                        color = match lang {
                            Lang::Json if matches!(word, "true" | "false" | "null") => Some(p.json_kw),
                            Lang::JavaScript if matches!(word, "true" | "false" | "null" | "undefined") => Some(p.json_num),
                            Lang::JavaScript if JS_KEYWORDS.contains(&word) => Some(p.json_kw),
                            Lang::JavaScript if word == "pm" || word == "console" => Some(p.accent),
                            Lang::GraphQL if GQL_KEYWORDS.contains(&word) => Some(p.json_kw),
                            Lang::GraphQL if word.starts_with('$') => Some(p.json_str),
                            _ => None,
                        };
                        if color.is_none() {
                            continue; // identifiers stay in the plain run
                        }
                    }
                }
                b'{' | b'}' | b'[' | b']' | b':' | b',' if lang == Lang::Json => {
                    i += 1;
                    color = Some(p.punct);
                }
                _ => {}
            }
        }
        match color {
            Some(col) => {
                flush(&mut job, plain_start, start, p.text);
                flush(&mut job, start, i, col);
                plain_start = i;
            }
            None => {
                i = start + 1;
                while i < bytes.len() && !text.is_char_boundary(i) {
                    i += 1;
                }
            }
        }
    }
    flush(&mut job, plain_start, bytes.len(), p.text);
    job
}

/// Multi-line code editor with syntax + variable highlighting.
pub fn code_editor(ui: &mut Ui, id: egui::Id, text: &mut String, lang: Lang, rows: usize, scope: Option<&VarScope>) -> egui::Response {
    let p = pal(ui);
    let font = FontId::monospace(13.0);
    let mut layouter = |ui: &Ui, s: &str, wrap: f32| -> Arc<Galley> {
        let mut job = highlight(s, lang, &p, &font, scope);
        job.wrap.max_width = wrap;
        ui.fonts(|f| f.layout_job(job))
    };
    egui::TextEdit::multiline(text)
        .id(id)
        .code_editor()
        .desired_rows(rows)
        .desired_width(f32::INFINITY)
        .lock_focus(true)
        .layouter(&mut layouter)
        .show(ui)
        .response
}

/// Single-line input that colours `{{vars}}` green (resolved) / red (missing) and shows values on hover.
pub fn var_input(ui: &mut Ui, text: &mut String, hint: &str, width: f32, scope: &VarScope, id: Option<egui::Id>) -> egui::Response {
    let p = pal(ui);
    let font = FontId::monospace(13.0);
    let has_vars = text.contains("{{");
    let mut layouter = |ui: &Ui, s: &str, wrap: f32| -> Arc<Galley> {
        let mut job = highlight(s, Lang::Plain, &p, &font, Some(scope));
        job.wrap.max_width = wrap;
        ui.fonts(|f| f.layout_job(job))
    };
    let mut te = egui::TextEdit::singleline(text).hint_text(hint).desired_width(width).font(font.clone());
    if let Some(id) = id {
        te = te.id(id);
    }
    if has_vars {
        te = te.layouter(&mut layouter);
    }
    let resp = ui.add(te);
    if has_vars {
        resp.on_hover_ui(|ui| var_tooltip(ui, text, scope))
    } else {
        resp
    }
}

pub fn var_tooltip(ui: &mut Ui, text: &str, scope: &VarScope) {
    let p = pal(ui);
    ui.set_max_width(520.0);
    ui.label(RichText::new(scope.resolve(text)).monospace());
    ui.separator();
    let mut seen = Vec::new();
    for (_, name) in find_vars(text) {
        if seen.contains(&name) {
            continue;
        }
        match scope.lookup(&name) {
            Some((v, src)) => {
                ui.label(RichText::new(format!("{name} = {v}")).monospace().color(p.ok));
                ui.label(dim(ui, format!("   from {}", src.label())).size(11.0));
            }
            None => {
                ui.label(RichText::new(format!("{name} is not defined")).monospace().color(p.err));
            }
        }
        seen.push(name);
    }
}

// ------------------------------------------------------------------ key/value table

pub struct KvOptions<'a> {
    pub allow_files: bool,
    pub secrets: bool,
    pub key_hint: &'a str,
    pub value_hint: &'a str,
    pub scope: Option<&'a VarScope>,
}

impl Default for KvOptions<'_> {
    fn default() -> Self {
        Self { allow_files: false, secrets: false, key_hint: "Key", value_hint: "Value", scope: None }
    }
}

fn to_bulk(rows: &[KeyValue]) -> String {
    rows.iter()
        .filter(|r| !r.is_blank())
        .map(|r| format!("{}{}: {}", if r.enabled { "" } else { "//" }, r.key, r.value))
        .collect::<Vec<_>>()
        .join("\n")
}

fn from_bulk(text: &str, old: &[KeyValue]) -> Vec<KeyValue> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (enabled, l) = match l.trim_start().strip_prefix("//") {
                Some(rest) => (false, rest),
                None => (true, l),
            };
            let (k, v) = l.split_once(':').unwrap_or((l, ""));
            let key = k.trim().to_string();
            let prev = old.iter().find(|o| o.key == key);
            KeyValue {
                key,
                value: v.trim().to_string(),
                enabled,
                description: prev.map(|p| p.description.clone()).unwrap_or_default(),
                kind: prev.map(|p| p.kind).unwrap_or_default(),
                secret: prev.is_some_and(|p| p.secret),
            }
        })
        .collect()
}

/// Editable key/value list with a trailing "ghost" row for adding entries. Returns true when changed.
pub fn kv_table(ui: &mut Ui, id_salt: impl std::hash::Hash, rows: &mut Vec<KeyValue>, opts: KvOptions) -> bool {
    let p = pal(ui);
    let id = ui.make_persistent_id(id_salt);
    let mut changed = false;
    let mut bulk: Option<String> = ui.data_mut(|d| d.get_temp::<Option<String>>(id).flatten());

    ui.horizontal(|ui| {
        let enabled = rows.iter().filter(|r| r.enabled && !r.key.is_empty()).count();
        ui.label(dim(ui, format!("{enabled} active")).size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if bulk.is_some() { "Key-value edit" } else { "Bulk edit" };
            if ui.add(egui::Button::new(RichText::new(label).size(11.5)).small().frame(false)).clicked() {
                bulk = match bulk.take() {
                    Some(text) => {
                        *rows = from_bulk(&text, rows);
                        changed = true;
                        None
                    }
                    None => Some(to_bulk(rows)),
                };
            }
        });
    });

    if let Some(text) = bulk.as_mut() {
        ui.label(dim(ui, "One per line as  key: value  — prefix with // to disable").size(11.0));
        let resp = egui::TextEdit::multiline(text)
            .code_editor()
            .desired_rows(8)
            .desired_width(f32::INFINITY)
            .show(ui)
            .response;
        if resp.changed() {
            *rows = from_bulk(text, rows);
            changed = true;
        }
        ui.data_mut(|d| d.insert_temp(id, bulk));
        return changed;
    }
    ui.data_mut(|d| d.insert_temp(id, None::<String>));

    let full = ui.available_width();
    let extra = if opts.allow_files { 62.0 } else { 0.0 } + if opts.secrets { 26.0 } else { 0.0 };
    let key_w = ((full - 24.0 - 30.0 - extra) * 0.36).max(80.0);
    let val_w = (full - key_w - 24.0 - 30.0 - extra - 5.0 * ui.spacing().item_spacing.x).max(80.0);
    let mut remove = None;
    let n = rows.len();
    for i in 0..=n {
        let ghost = i == n;
        let mut tmp = KeyValue::default();
        let row: &mut KeyValue = if ghost { &mut tmp } else { &mut rows[i] };
        let mut row_changed = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!ghost, |ui| {
                if ui.checkbox(&mut row.enabled, "").changed() {
                    row_changed = true;
                }
            });
            let key_color = if row.enabled { p.text } else { p.dim };
            if ui
                .add(
                    egui::TextEdit::singleline(&mut row.key)
                        .hint_text(opts.key_hint)
                        .desired_width(key_w)
                        .text_color(key_color)
                        .font(egui::TextStyle::Monospace),
                )
                .changed()
            {
                row_changed = true;
            }
            if opts.allow_files {
                egui::ComboBox::from_id_salt((id, i, "kind"))
                    .width(52.0)
                    .selected_text(if row.kind == KvKind::File { "File" } else { "Text" })
                    .show_ui(ui, |ui| {
                        row_changed |= ui.selectable_value(&mut row.kind, KvKind::Text, "Text").changed();
                        row_changed |= ui.selectable_value(&mut row.kind, KvKind::File, "File").changed();
                    });
            }
            if row.kind == KvKind::File {
                let label = if row.value.is_empty() {
                    "Choose file…".to_string()
                } else {
                    std::path::Path::new(&row.value).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(row.value.clone())
                };
                if ui.add_sized([val_w, 24.0], egui::Button::new(label)).on_hover_text(&row.value).clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        row.value = path.display().to_string();
                        row_changed = true;
                    }
                }
            } else {
                let masked = opts.secrets && row.secret;
                let resp = match (opts.scope, masked) {
                    (Some(scope), false) => var_input(ui, &mut row.value, opts.value_hint, val_w, scope, None),
                    _ => ui.add(
                        egui::TextEdit::singleline(&mut row.value)
                            .hint_text(opts.value_hint)
                            .desired_width(val_w)
                            .password(masked)
                            .font(egui::TextStyle::Monospace),
                    ),
                };
                if resp.changed() {
                    row_changed = true;
                }
            }
            if opts.secrets && !ghost {
                let icon = if row.secret { "🔒" } else { "🔓" };
                if ui.add(egui::Button::new(icon).frame(false)).on_hover_text("Mark as secret (masked)").clicked() {
                    row.secret = !row.secret;
                    row_changed = true;
                }
            }
            if !ghost && ui.add(egui::Button::new(RichText::new("✕").color(p.dim)).frame(false)).on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
        });
        if row_changed {
            changed = true;
            if ghost && !tmp.is_blank() {
                rows.push(tmp);
            }
        }
    }
    if let Some(i) = remove {
        rows.remove(i);
        changed = true;
    }
    changed
}

/// A labelled form row: fixed-width label then the widget.
pub fn form_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(130.0, 22.0), Sense::hover());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, FontId::proportional(13.0), pal(ui).dim);
        add(ui)
    })
    .inner
}

pub fn empty_state(ui: &mut Ui, icon: &str, title: &str, subtitle: &str) {
    let p = pal(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.25).max(10.0));
        ui.label(RichText::new(icon).size(34.0).color(p.dim));
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(15.0).color(p.text));
        ui.label(RichText::new(subtitle).color(p.dim));
    });
}

/// Empty state with the Tessera logo mark.
pub fn logo_empty_state(ui: &mut Ui, title: &str, subtitle: &str) {
    let p = pal(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(10.0));
        ui.add(egui::Image::from_bytes("bytes://tessera-icon.png", crate::theme::ICON_PNG).fit_to_exact_size(Vec2::splat(56.0)).tint(egui::Color32::from_white_alpha(230)));
        ui.add_space(8.0);
        ui.label(RichText::new(title).size(15.0).color(p.text));
        ui.label(RichText::new(subtitle).color(p.dim));
    });
}
