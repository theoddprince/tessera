//! Response pane.

use egui::{FontId, RichText, Ui};
use serde_json::Value;

use crate::state::{BodyMode, RespSection, ResponseState, Tab};
use crate::theme;
use crate::ui::widgets::{self, dim, highlight, pal, Lang};
use crate::view::BodyKind;

pub fn show(ui: &mut Ui, tab: &mut Tab) {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;

    if let Some(pending) = &tab.pending {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Sending… {}", widgets::human_time(pending.started.elapsed().as_secs_f64() * 1000.0)));
            ui.label(dim(ui, "Esc to cancel"));
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        if tab.response.is_none() {
            return;
        }
    }
    let Some(resp) = tab.response.as_mut() else {
        widgets::logo_empty_state(ui, "Ready when you are", "Enter a URL and hit Send  (Ctrl+Enter)");
        return;
    };

    // ---- status line ----
    let mut copy_body = false;
    let mut save_body = false;
    ui.horizontal(|ui| {
        match &resp.result {
            Ok(r) => {
                widgets::chip(ui, format!("{} {}", r.status, r.status_text), theme::status_color(r.status, dark));
                ui.label(RichText::new(widgets::human_time(r.time_ms)).color(p.text))
                    .on_hover_text(format!("Time to first byte: {}\nDownload: {}", widgets::human_time(r.ttfb_ms), widgets::human_time(r.time_ms - r.ttfb_ms)));
                ui.label(dim(ui, "·"));
                ui.label(widgets::human_size(r.size())).on_hover_text(format!("Body {} · Headers {}", widgets::human_size(r.body.len()), widgets::human_size(r.size() - r.body.len())));
                ui.label(dim(ui, "·"));
                ui.label(dim(ui, &r.http_version));
                if r.final_url != r.request_url {
                    ui.label(dim(ui, "· redirected")).on_hover_text(&r.final_url);
                }
            }
            Err(e) => {
                widgets::chip(ui, "Error", p.err);
                ui.label(RichText::new(e).color(p.err));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if resp.result.is_ok() {
                if ui.small_button("Save…").on_hover_text("Save body to file").clicked() {
                    save_body = true;
                }
                if ui.small_button("Copy").on_hover_text("Copy body").clicked() {
                    copy_body = true;
                }
            }
        });
    });
    if copy_body {
        if let Ok(r) = &resp.result {
            ui.ctx().copy_text(r.text());
        }
    }
    if save_body {
        if let Ok(r) = &resp.result {
            if let Some(path) = rfd::FileDialog::new().set_file_name("response").save_file() {
                let _ = std::fs::write(path, &r.body);
            }
        }
    }

    let (n_headers, n_cookies) = resp.result.as_ref().map(|r| (r.headers.len(), r.cookies.len())).unwrap_or((0, 0));
    let passed = resp.tests.iter().filter(|t| t.passed).count();
    let tests_label = if resp.tests.is_empty() { "Tests".to_string() } else { format!("Tests {passed}/{}", resp.tests.len()) };
    let console_label = if resp.console.is_empty() { "Console".to_string() } else { format!("Console ({})", resp.console.len()) };
    ui.add_space(2.0);
    widgets::tab_strip(
        ui,
        &mut tab.resp_section,
        &[
            (RespSection::Body, "Body".into()),
            (RespSection::Headers, format!("Headers ({n_headers})")),
            (RespSection::Cookies, format!("Cookies ({n_cookies})")),
            (RespSection::Tests, tests_label),
            (RespSection::Console, console_label),
            (RespSection::Request, "Request".into()),
        ],
    );
    ui.add_space(6.0);

    match tab.resp_section {
        RespSection::Body => body(ui, resp, &mut tab.body_mode),
        RespSection::Headers => {
            if let Ok(r) = &resp.result {
                kv_grid(ui, "resp-headers", &r.headers);
            }
        }
        RespSection::Cookies => cookies(ui, resp),
        RespSection::Tests => tests(ui, resp),
        RespSection::Console => console(ui, resp),
        RespSection::Request => request_info(ui, resp),
    }
}

fn kv_grid(ui: &mut Ui, id: &str, rows: &[(String, String)]) {
    let p = pal(ui);
    egui::ScrollArea::both().id_salt(id).auto_shrink([false; 2]).show(ui, |ui| {
        egui::Grid::new(id).striped(true).num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
            for (k, v) in rows {
                ui.add(egui::Label::new(RichText::new(k).monospace().color(p.json_key)).selectable(true));
                ui.add(egui::Label::new(RichText::new(v).monospace()).selectable(true).wrap());
                ui.end_row();
            }
        });
    });
}

fn cookies(ui: &mut Ui, resp: &ResponseState) {
    let Ok(r) = &resp.result else { return };
    if r.cookies.is_empty() {
        widgets::empty_state(ui, "🍪", "No cookies", "This response didn't set any cookies");
        return;
    }
    egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
        egui::Grid::new("cookies").striped(true).num_columns(6).spacing([18.0, 6.0]).show(ui, |ui| {
            for h in ["Name", "Value", "Domain", "Path", "Expires", "Flags"] {
                ui.label(dim(ui, h).strong());
            }
            ui.end_row();
            for c in &r.cookies {
                ui.add(egui::Label::new(RichText::new(&c.name).monospace()).selectable(true));
                ui.add(egui::Label::new(RichText::new(&c.value).monospace()).selectable(true));
                ui.label(&c.domain);
                ui.label(&c.path);
                ui.label(&c.expires);
                let mut flags = Vec::new();
                if c.http_only {
                    flags.push("HttpOnly");
                }
                if c.secure {
                    flags.push("Secure");
                }
                ui.label(flags.join(", "));
                ui.end_row();
            }
        });
    });
}

fn tests(ui: &mut Ui, resp: &ResponseState) {
    let p = pal(ui);
    if resp.tests.is_empty() {
        widgets::empty_state(ui, "🧪", "No tests", "Write tests in the Scripts tab, e.g.  pm.test(\"ok\", () => pm.response.to.have.status(200))");
        return;
    }
    let passed = resp.tests.iter().filter(|t| t.passed).count();
    ui.horizontal(|ui| {
        widgets::chip(ui, format!("{passed} passed"), p.ok);
        if passed < resp.tests.len() {
            widgets::chip(ui, format!("{} failed", resp.tests.len() - passed), p.err);
        }
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
        for t in &resp.tests {
            ui.horizontal(|ui| {
                if t.passed {
                    ui.label(RichText::new("✔").color(p.ok));
                } else {
                    ui.label(RichText::new("✖").color(p.err));
                }
                ui.label(&t.name);
            });
            if !t.passed && !t.error.is_empty() {
                ui.indent("err", |ui| {
                    ui.label(RichText::new(&t.error).monospace().color(p.err).size(12.0));
                });
            }
        }
    });
}

fn console(ui: &mut Ui, resp: &ResponseState) {
    let p = pal(ui);
    if resp.console.is_empty() {
        widgets::empty_state(ui, ">_", "Console is empty", "console.log() output from scripts appears here");
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
        for line in &resp.console {
            let color = match line.level.as_str() {
                "error" => p.err,
                "warn" => p.warn,
                "info" => p.info,
                _ => p.text,
            };
            ui.add(egui::Label::new(RichText::new(&line.text).monospace().color(color)).selectable(true).wrap());
            ui.separator();
        }
    });
}

fn request_info(ui: &mut Ui, resp: &ResponseState) {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    let Some(prep) = &resp.prepared else {
        ui.label(dim(ui, "No request details"));
        return;
    };
    ui.horizontal(|ui| {
        ui.label(widgets::method_text(&prep.method, dark, 13.0));
        ui.add(egui::Label::new(RichText::new(&prep.url).monospace()).selectable(true).wrap());
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Request headers").color(p.dim));
    kv_grid(ui, "req-headers", &prep.headers);
}

fn body(ui: &mut Ui, resp: &mut ResponseState, mode: &mut BodyMode) {
    let p = pal(ui);
    let kind = resp.view.kind;
    if kind == BodyKind::Empty {
        if let Ok(r) = &resp.result {
            widgets::empty_state(ui, "∅", "Empty body", &format!("{} {}", r.status, r.status_text));
        }
        return;
    }

    let mut modes: Vec<(BodyMode, &str)> = vec![(BodyMode::Pretty, "Pretty"), (BodyMode::Raw, "Raw")];
    if resp.view.json.is_some() {
        modes.push((BodyMode::Tree, "Tree"));
    }
    if kind == BodyKind::Image {
        modes = vec![(BodyMode::Preview, "Preview")];
    }
    if !modes.iter().any(|(m, _)| m == mode) {
        *mode = modes[0].0;
    }

    let lines_len = match mode {
        BodyMode::Raw => resp.view.raw.len(),
        _ => resp.view.pretty.len(),
    };
    ui.horizontal(|ui| {
        widgets::segmented(ui, mode, &modes);
        ui.label(dim(ui, kind.label()).size(12.0));
        if matches!(mode, BodyMode::Pretty | BodyMode::Raw) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let lines = if *mode == BodyMode::Raw { &resp.view.raw } else { &resp.view.pretty };
                let q = resp.search.to_lowercase();
                let hits: Vec<usize> = if q.is_empty() {
                    Vec::new()
                } else {
                    lines.iter().enumerate().filter(|(_, l)| l.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
                };
                if !hits.is_empty() {
                    if ui.small_button("▼").on_hover_text("Next match (Enter)").clicked() {
                        resp.search_hit = (resp.search_hit + 1) % hits.len();
                        resp.scroll_to = Some(hits[resp.search_hit]);
                    }
                    if ui.small_button("▲").on_hover_text("Previous match").clicked() {
                        resp.search_hit = (resp.search_hit + hits.len() - 1) % hits.len();
                        resp.scroll_to = Some(hits[resp.search_hit]);
                    }
                    ui.label(dim(ui, format!("{}/{}", resp.search_hit.min(hits.len() - 1) + 1, hits.len())).size(12.0));
                } else if !q.is_empty() {
                    ui.label(RichText::new("no matches").color(p.err).size(12.0));
                }
                let r = ui.add(egui::TextEdit::singleline(&mut resp.search).hint_text("🔍 Find in body").desired_width(180.0));
                if r.changed() {
                    resp.search_hit = 0;
                    let q = resp.search.to_lowercase();
                    resp.scroll_to = lines.iter().position(|l| !q.is_empty() && l.to_lowercase().contains(&q));
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !hits.is_empty() {
                    resp.search_hit = (resp.search_hit + 1) % hits.len();
                    resp.scroll_to = Some(hits[resp.search_hit]);
                    r.request_focus();
                }
            });
        }
    });
    ui.add_space(4.0);

    match mode {
        BodyMode::Preview => {
            if let Some((uri, bytes)) = &resp.view.image {
                egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
                    ui.add(egui::Image::from_bytes(uri.clone(), bytes.clone()).max_width(ui.available_width()).fit_to_original_size(1.0));
                });
            }
        }
        BodyMode::Tree => {
            if let Some(v) = &resp.view.json {
                egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 1.0);
                    ui.spacing_mut().interact_size.y = 20.0;
                    json_tree(ui, None, v, "$", 0);
                });
            }
        }
        BodyMode::Pretty | BodyMode::Raw => {
            let lines = if *mode == BodyMode::Raw { &resp.view.raw } else { &resp.view.pretty };
            let lang = match (kind, *mode) {
                (BodyKind::Json, BodyMode::Pretty) => Lang::Json,
                (BodyKind::Xml | BodyKind::Html, BodyMode::Pretty) => Lang::Markup,
                _ => Lang::Plain,
            };
            code_lines(ui, lines, lang, &resp.search, resp.scroll_to.take(), lines_len);
        }
    }
}

/// Virtualised, highlighted, line-numbered text view — renders only visible rows.
fn code_lines(ui: &mut Ui, lines: &[String], lang: Lang, search: &str, scroll_to: Option<usize>, total: usize) {
    let p = pal(ui);
    let font = FontId::monospace(13.0);
    let row_h = ui.fonts(|f| f.row_height(&font)) + 2.0;
    let gutter_chars = total.max(1).to_string().len();
    let q = search.to_lowercase();
    let frame = egui::Frame::new().fill(p.surface).corner_radius(6).inner_margin(egui::Margin::same(6));
    frame.show(ui, |ui| {
        let mut area = egui::ScrollArea::both().auto_shrink([false; 2]);
        if let Some(line) = scroll_to {
            area = area.vertical_scroll_offset((line as f32 * row_h - 60.0).max(0.0));
        }
        area.show_rows(ui, row_h, lines.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let gutter_w = ui.fonts(|f| f.glyph_width(&font, '0')) * gutter_chars as f32 + 12.0;
            for i in range {
                let line = &lines[i];
                let mut job = highlight(line, lang, &p, &font, None);
                job.wrap.max_width = f32::INFINITY;
                if !q.is_empty() && line.to_lowercase().contains(&q) {
                    for s in job.sections.iter_mut() {
                        s.format.background = p.warn.gamma_multiply(0.25);
                    }
                }
                let galley = ui.fonts(|f| f.layout_job(job));
                let width = (gutter_w + galley.size().x + 8.0).max(ui.available_width());
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, row_h), egui::Sense::hover());
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    painter.text(
                        egui::pos2(rect.left() + gutter_w - 12.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        (i + 1).to_string(),
                        font.clone(),
                        p.dim.gamma_multiply(0.7),
                    );
                    painter.galley(egui::pos2(rect.left() + gutter_w, rect.top() + 1.0), galley, p.text);
                }
            }
        });
    });
}

const TREE_PAGE: usize = 200;

fn json_tree(ui: &mut Ui, key: Option<&str>, v: &Value, path: &str, depth: usize) {
    let p = pal(ui);
    let key_text = |ui: &mut Ui| {
        if let Some(k) = key {
            ui.label(RichText::new(format!("{k}:")).monospace().color(p.json_key));
        }
    };
    match v {
        Value::Object(map) => {
            let id = ui.make_persistent_id(path);
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < 2)
                .show_header(ui, |ui| {
                    key_text(ui);
                    ui.label(RichText::new(format!("{{{}}}", map.len())).monospace().color(p.dim));
                })
                .body(|ui| {
                    let limit_id = id.with("limit");
                    let limit = ui.data(|d| d.get_temp::<usize>(limit_id)).unwrap_or(TREE_PAGE);
                    for (k, child) in map.iter().take(limit) {
                        let child_path = if k.chars().all(|c| c.is_alphanumeric() || c == '_') { format!("{path}.{k}") } else { format!("{path}[{k:?}]") };
                        json_tree(ui, Some(k), child, &child_path, depth + 1);
                    }
                    if map.len() > limit && ui.small_button(format!("Show {} more…", (map.len() - limit).min(TREE_PAGE))).clicked() {
                        ui.data_mut(|d| d.insert_temp(limit_id, limit + TREE_PAGE));
                    }
                })
                .0
                .context_menu(|ui| tree_menu(ui, path, v));
        }
        Value::Array(arr) => {
            let id = ui.make_persistent_id(path);
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < 2)
                .show_header(ui, |ui| {
                    key_text(ui);
                    ui.label(RichText::new(format!("[{}]", arr.len())).monospace().color(p.dim));
                })
                .body(|ui| {
                    let limit_id = id.with("limit");
                    let limit = ui.data(|d| d.get_temp::<usize>(limit_id)).unwrap_or(TREE_PAGE);
                    for (i, child) in arr.iter().enumerate().take(limit) {
                        json_tree(ui, Some(&i.to_string()), child, &format!("{path}[{i}]"), depth + 1);
                    }
                    if arr.len() > limit && ui.small_button(format!("Show {} more…", (arr.len() - limit).min(TREE_PAGE))).clicked() {
                        ui.data_mut(|d| d.insert_temp(limit_id, limit + TREE_PAGE));
                    }
                })
                .0
                .context_menu(|ui| tree_menu(ui, path, v));
        }
        leaf => {
            let (text, color) = match leaf {
                Value::String(s) => (format!("{s:?}"), p.json_str),
                Value::Number(n) => (n.to_string(), p.json_num),
                Value::Bool(b) => (b.to_string(), p.json_kw),
                _ => ("null".to_string(), p.json_kw),
            };
            let mut job = egui::text::LayoutJob::default();
            let font = egui::FontId::monospace(13.0);
            if let Some(k) = key {
                job.append(&format!("{k}: "), 18.0, egui::TextFormat::simple(font.clone(), p.json_key));
            }
            job.append(&text, if key.is_some() { 0.0 } else { 18.0 }, egui::TextFormat::simple(font, color));
            let r = ui.add(egui::Label::new(job).truncate().selectable(false).sense(egui::Sense::click()));
            r.on_hover_text(format!("{path}\nRight-click to copy")).context_menu(|ui| tree_menu(ui, path, v));
        }
    }
}

fn tree_menu(ui: &mut Ui, path: &str, v: &Value) {
    if ui.button("Copy path").clicked() {
        ui.ctx().copy_text(path.to_string());
        ui.close_menu();
    }
    if ui.button("Copy value").clicked() {
        let text = match v {
            Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        };
        ui.ctx().copy_text(text);
        ui.close_menu();
    }
    if ui.button("Copy as pm.expect").clicked() {
        let js_path = path.trim_start_matches('$');
        ui.ctx().copy_text(format!("pm.expect(pm.response.json(){js_path}).to.eql({});", serde_json::to_string(v).unwrap_or_default()));
        ui.close_menu();
    }
}
