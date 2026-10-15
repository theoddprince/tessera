//! Sidebar: collections tree and history.

use std::collections::HashSet;

use egui::{Align2, FontId, RichText, Sense, Ui, Vec2};
use tessera_core::model::{Collection, HistoryEntry, Item, RequestKind};

use crate::app::Action;
use crate::theme;
use crate::ui::widgets::{dim, pal};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarMode {
    #[default]
    Collections,
    Flows,
    History,
}

/// Item being dragged in the tree.
#[derive(Clone, Debug)]
pub struct DragItem {
    pub collection_id: String,
    pub item_id: String,
}

pub struct SidebarCtx<'a> {
    pub expanded: &'a mut HashSet<String>,
    pub renaming: &'a mut Option<(String, String)>,
    pub filter: &'a str,
    pub active_request: Option<&'a str>,
    pub actions: &'a mut Vec<Action>,
}

const ROW_H: f32 = 26.0;

enum RowKind<'a> {
    Collection,
    Folder,
    Request { badge: &'a str },
}

struct RowOut {
    clicked: bool,
    response: egui::Response,
}

#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, depth: usize, kind: RowKind, label: &str, id: &str, open: bool, selected: bool, ctx: &mut SidebarCtx) -> RowOut {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 5.0, p.accent.gamma_multiply(0.18));
    } else if response.hovered() {
        painter.rect_filled(rect, 5.0, p.surface_hi);
    }
    let mut x = rect.left() + 8.0 + depth as f32 * 14.0;
    let cy = rect.center().y;
    match kind {
        RowKind::Collection | RowKind::Folder => {
            painter.text(egui::pos2(x, cy), Align2::LEFT_CENTER, if open { "▾" } else { "▸" }, FontId::proportional(12.0), p.dim);
            x += 14.0;
            let icon = if matches!(kind, RowKind::Collection) { "🗄" } else if open { "📂" } else { "📁" };
            painter.text(egui::pos2(x, cy), Align2::LEFT_CENTER, icon, FontId::proportional(13.0), if matches!(kind, RowKind::Collection) { p.accent } else { p.dim });
            x += 20.0;
        }
        RowKind::Request { badge } => {
            x += 14.0;
            let short = match badge {
                "DELETE" => "DEL",
                "OPTIONS" => "OPT",
                "PATCH" => "PTCH",
                b => b,
            };
            painter.text(egui::pos2(x, cy), Align2::LEFT_CENTER, short, FontId::monospace(10.5), theme::method_color(badge, dark));
            x += 40.0;
        }
    }

    let renaming = ctx.renaming.as_ref().is_some_and(|(rid, _)| rid == id);
    if renaming {
        let text_rect = egui::Rect::from_min_max(egui::pos2(x, rect.top() + 2.0), egui::pos2(rect.right() - 6.0, rect.bottom() - 2.0));
        let (_, buf) = ctx.renaming.as_mut().unwrap();
        let te = ui.put(text_rect, egui::TextEdit::singleline(buf).desired_width(text_rect.width()));
        te.request_focus();
        if te.lost_focus() {
            let commit = !ui.input(|i| i.key_pressed(egui::Key::Escape));
            let (rid, name) = ctx.renaming.take().unwrap();
            if commit && !name.trim().is_empty() {
                ctx.actions.push(Action::Rename { id: rid, name: name.trim().to_string() });
            }
        }
    } else {
        let font = FontId::proportional(13.0);
        let max_w = rect.right() - x - 26.0;
        let galley = ui.fonts(|f| {
            let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font.clone(), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
            f.layout_job(job)
        });
        painter.galley(egui::pos2(x, cy - galley.size().y / 2.0), galley, p.text);
        if response.hovered() {
            painter.text(egui::pos2(rect.right() - 10.0, cy), Align2::RIGHT_CENTER, "⋯", FontId::proportional(14.0), p.dim);
        }
    }

    // Drop target highlight.
    if response.dnd_hover_payload::<DragItem>().is_some() {
        let line_y = if matches!(kind, RowKind::Request { .. }) { rect.top() } else { rect.bottom() };
        if matches!(kind, RowKind::Request { .. }) {
            painter.line_segment([egui::pos2(rect.left(), line_y), egui::pos2(rect.right(), line_y)], egui::Stroke::new(2.0_f32, p.accent));
        } else {
            painter.rect_stroke(rect, 5.0, egui::Stroke::new(1.5_f32, p.accent), egui::StrokeKind::Inside);
        }
    }
    RowOut { clicked: response.clicked() && !renaming, response }
}

fn matches_filter(item: &Item, f: &str) -> bool {
    if f.is_empty() {
        return true;
    }
    match item {
        Item::Request(r) => r.name.to_lowercase().contains(f) || r.url.to_lowercase().contains(f),
        Item::Folder(fd) => fd.name.to_lowercase().contains(f) || fd.items.iter().any(|i| matches_filter(i, f)),
    }
}

pub fn collections(ui: &mut Ui, cols: &[Collection], ctx: &mut SidebarCtx) {
    let f = ctx.filter.to_lowercase();
    if cols.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🗄").size(28.0).color(pal(ui).dim));
            ui.label("No collections yet");
            ui.label(dim(ui, "Create one or import from Postman"));
            ui.add_space(6.0);
            if ui.button("➕ New collection").clicked() {
                ctx.actions.push(Action::NewCollection);
            }
            if ui.button("⤓ Import…").clicked() {
                ctx.actions.push(Action::OpenImport);
            }
        });
        return;
    }
    for c in cols {
        if !f.is_empty() && !c.name.to_lowercase().contains(&f) && !c.items.iter().any(|i| matches_filter(i, &f)) {
            continue;
        }
        let open = ctx.expanded.contains(&c.id) || !f.is_empty();
        let out = row(ui, 0, RowKind::Collection, &c.name, &c.id, open, false, ctx);
        if out.clicked {
            toggle(ctx.expanded, &c.id);
        }
        if out.response.double_clicked() {
            ctx.actions.push(Action::ItemSettings { collection_id: c.id.clone(), folder_id: None });
        }
        if let Some(drag) = out.response.dnd_release_payload::<DragItem>() {
            ctx.actions.push(Action::Move { from_collection: drag.collection_id.clone(), item_id: drag.item_id.clone(), to_collection: c.id.clone(), container: c.id.clone(), index: None });
        }
        out.response.context_menu(|ui| container_menu(ui, c, None, ctx));
        if open {
            items(ui, c, &c.items, 1, &f, ctx);
            if c.items.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    if ui.add(egui::Button::new(dim(ui, "+ Add a request").size(12.0)).frame(false)).clicked() {
                        ctx.actions.push(Action::AddRequest { collection_id: c.id.clone(), container: c.id.clone(), kind: RequestKind::Http });
                    }
                });
            }
        }
    }
}

fn toggle(set: &mut HashSet<String>, id: &str) {
    if !set.remove(id) {
        set.insert(id.to_string());
    }
}

fn items(ui: &mut Ui, c: &Collection, list: &[Item], depth: usize, f: &str, ctx: &mut SidebarCtx) {
    for (index, item) in list.iter().enumerate() {
        if !matches_filter(item, f) {
            continue;
        }
        match item {
            Item::Folder(fd) => {
                let open = ctx.expanded.contains(&fd.id) || !f.is_empty();
                let out = row(ui, depth, RowKind::Folder, &fd.name, &fd.id, open, false, ctx);
                if out.clicked {
                    toggle(ctx.expanded, &fd.id);
                }
                if out.response.double_clicked() {
                    ctx.actions.push(Action::ItemSettings { collection_id: c.id.clone(), folder_id: Some(fd.id.clone()) });
                }
                drag_source(&out.response, c, &fd.id, &fd.name);
                if let Some(drag) = out.response.dnd_release_payload::<DragItem>() {
                    if drag.item_id != fd.id {
                        ctx.actions.push(Action::Move { from_collection: drag.collection_id.clone(), item_id: drag.item_id.clone(), to_collection: c.id.clone(), container: fd.id.clone(), index: None });
                    }
                }
                out.response.context_menu(|ui| container_menu(ui, c, Some((&fd.id, &fd.name)), ctx));
                if open {
                    items(ui, c, &fd.items, depth + 1, f, ctx);
                }
            }
            Item::Request(r) => {
                let selected = ctx.active_request == Some(r.id.as_str());
                let out = row(ui, depth, RowKind::Request { badge: r.badge() }, &r.name, &r.id, false, selected, ctx);
                if out.clicked {
                    ctx.actions.push(Action::OpenRequest { collection_id: c.id.clone(), request_id: r.id.clone() });
                }
                drag_source(&out.response, c, &r.id, &r.name);
                if let Some(drag) = out.response.dnd_release_payload::<DragItem>() {
                    if drag.item_id != r.id {
                        let container = c.locate(&r.id).map(|(cid, _)| cid).unwrap_or(c.id.clone());
                        ctx.actions.push(Action::Move { from_collection: drag.collection_id.clone(), item_id: drag.item_id.clone(), to_collection: c.id.clone(), container, index: Some(index) });
                    }
                }
                let resp = out.response.on_hover_text_at_pointer(format!("{} {}", r.badge(), r.url));
                resp.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        ctx.actions.push(Action::OpenRequest { collection_id: c.id.clone(), request_id: r.id.clone() });
                        ui.close_menu();
                    }
                    if ui.button("Rename").clicked() {
                        *ctx.renaming = Some((r.id.clone(), r.name.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Duplicate").clicked() {
                        ctx.actions.push(Action::Duplicate { id: r.id.clone() });
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(RichText::new("Delete").color(pal(ui).err)).clicked() {
                        ctx.actions.push(Action::AskDelete { id: r.id.clone(), name: r.name.clone() });
                        ui.close_menu();
                    }
                });
            }
        }
    }
}

fn drag_source(resp: &egui::Response, c: &Collection, id: &str, name: &str) {
    if resp.dragged() {
        resp.dnd_set_drag_payload(DragItem { collection_id: c.id.clone(), item_id: id.to_string() });
        egui::show_tooltip_at_pointer(&resp.ctx, resp.layer_id, resp.id.with("drag"), |ui| {
            ui.label(format!("↕ {name}"));
        });
    }
}

fn container_menu(ui: &mut Ui, c: &Collection, folder: Option<(&String, &String)>, ctx: &mut SidebarCtx) {
    let container = folder.map(|(id, _)| id.clone()).unwrap_or(c.id.clone());
    if ui.button("➕ Add request").clicked() {
        ctx.actions.push(Action::AddRequest { collection_id: c.id.clone(), container: container.clone(), kind: RequestKind::Http });
        ui.close_menu();
    }
    if ui.button("➕ Add GraphQL request").clicked() {
        ctx.actions.push(Action::AddRequest { collection_id: c.id.clone(), container: container.clone(), kind: RequestKind::GraphQL });
        ui.close_menu();
    }
    if ui.button("➕ Add WebSocket").clicked() {
        ctx.actions.push(Action::AddRequest { collection_id: c.id.clone(), container: container.clone(), kind: RequestKind::WebSocket });
        ui.close_menu();
    }
    if ui.button("📁 Add folder").clicked() {
        ctx.actions.push(Action::AddFolder { collection_id: c.id.clone(), container: container.clone() });
        ui.close_menu();
    }
    ui.separator();
    if ui.button("▶ Run").clicked() {
        ctx.actions.push(Action::OpenRunner { collection_id: c.id.clone(), folder_id: folder.map(|(id, _)| id.clone()) });
        ui.close_menu();
    }
    if ui.button("⚙ Settings (auth, scripts, variables)").clicked() {
        ctx.actions.push(Action::ItemSettings { collection_id: c.id.clone(), folder_id: folder.map(|(id, _)| id.clone()) });
        ui.close_menu();
    }
    if ui.button("✏ Rename").clicked() {
        *ctx.renaming = Some(match folder {
            Some((id, name)) => (id.clone(), name.clone()),
            None => (c.id.clone(), c.name.clone()),
        });
        ui.close_menu();
    }
    if ui.button("⧉ Duplicate").clicked() {
        ctx.actions.push(Action::Duplicate { id: container.clone() });
        ui.close_menu();
    }
    if folder.is_none() && ui.button("⤒ Export (Postman v2.1)…").clicked() {
        ctx.actions.push(Action::ExportCollection(c.id.clone()));
        ui.close_menu();
    }
    ui.separator();
    if ui.button(RichText::new("🗑 Delete").color(pal(ui).err)).clicked() {
        let name = folder.map(|(_, n)| n.clone()).unwrap_or(c.name.clone());
        ctx.actions.push(Action::AskDelete { id: container, name });
        ui.close_menu();
    }
}

pub fn history(ui: &mut Ui, entries: &[HistoryEntry], filter: &str, actions: &mut Vec<Action>) {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    if entries.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🕘").size(28.0).color(p.dim));
            ui.label("No history yet");
            ui.label(dim(ui, "Requests you send show up here"));
        });
        return;
    }
    let f = filter.to_lowercase();
    let today = chrono::Local::now().date_naive();
    let mut last_day = String::new();
    for (i, e) in entries.iter().enumerate() {
        if !f.is_empty() && !e.request.url.to_lowercase().contains(&f) && !e.request.name.to_lowercase().contains(&f) {
            continue;
        }
        let when = chrono::DateTime::parse_from_rfc3339(&e.timestamp).ok().map(|t| t.with_timezone(&chrono::Local));
        let day = match when {
            Some(t) if t.date_naive() == today => "Today".to_string(),
            Some(t) if Some(t.date_naive()) == today.pred_opt() => "Yesterday".to_string(),
            Some(t) => t.format("%a %d %b %Y").to_string(),
            None => "Earlier".to_string(),
        };
        if day != last_day {
            ui.add_space(4.0);
            ui.label(RichText::new(&day).size(11.0).color(p.dim).strong());
            last_day = day;
        }
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        let painter = ui.painter_at(rect);
        if resp.hovered() {
            painter.rect_filled(rect, 5.0, p.surface_hi);
        }
        let cy = rect.center().y;
        let badge = e.request.badge();
        painter.text(egui::pos2(rect.left() + 8.0, cy), Align2::LEFT_CENTER, if badge == "DELETE" { "DEL" } else { badge }, FontId::monospace(10.5), theme::method_color(badge, dark));
        let status_text = if e.status == 0 { "ERR".to_string() } else { e.status.to_string() };
        painter.text(egui::pos2(rect.right() - 6.0, cy), Align2::RIGHT_CENTER, &status_text, FontId::monospace(10.5), if e.status == 0 { p.err } else { theme::status_color(e.status, dark) });
        let url = e.request.url.split("://").nth(1).unwrap_or(&e.request.url);
        let galley = ui.fonts(|fo| {
            let mut job = egui::text::LayoutJob::simple_singleline(url.to_string(), FontId::proportional(12.5), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width((width - 90.0).max(20.0));
            fo.layout_job(job)
        });
        painter.galley(egui::pos2(rect.left() + 48.0, cy - galley.size().y / 2.0), galley, p.text);
        let resp = resp.on_hover_text(format!(
            "{} {}\n{} · {}",
            badge,
            e.request.url,
            when.map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default(),
            crate::ui::widgets::human_time(e.time_ms)
        ));
        if resp.clicked() {
            actions.push(Action::OpenHistory(i));
        }
        resp.context_menu(|ui| {
            if ui.button("Open in new tab").clicked() {
                actions.push(Action::OpenHistory(i));
                ui.close_menu();
            }
            if ui.button("Copy URL").clicked() {
                ui.ctx().copy_text(e.request.url.clone());
                ui.close_menu();
            }
            if ui.button("Delete").clicked() {
                actions.push(Action::DeleteHistory(i));
                ui.close_menu();
            }
        });
    }
}

pub fn flows(ui: &mut Ui, flows: &[tessera_core::flow::Flow], ctx: &mut SidebarCtx, active_flow: Option<&str>) {
    let p = pal(ui);
    if flows.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("⇢").size(28.0).color(p.dim));
            ui.label("No flows yet");
            ui.label(dim(ui, "Chain requests together, e.g. log in → use the token → fetch data"));
            ui.add_space(6.0);
            if ui.button("➕ New flow").clicked() {
                ctx.actions.push(Action::NewFlow);
            }
        });
        return;
    }
    let f = ctx.filter.to_lowercase();
    for flow in flows.iter().filter(|fl| f.is_empty() || fl.name.to_lowercase().contains(&f)) {
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H + 8.0), Sense::click());
        let painter = ui.painter_at(rect);
        if active_flow == Some(flow.id.as_str()) {
            painter.rect_filled(rect, 5.0, p.accent.gamma_multiply(0.18));
        } else if resp.hovered() {
            painter.rect_filled(rect, 5.0, p.surface_hi);
        }
        let renaming = ctx.renaming.as_ref().is_some_and(|(id, _)| id == &flow.id);
        painter.text(egui::pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, "⇢", FontId::proportional(15.0), p.accent);
        if renaming {
            let text_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 30.0, rect.top() + 4.0), egui::pos2(rect.right() - 6.0, rect.bottom() - 4.0));
            let (_, buf) = ctx.renaming.as_mut().unwrap();
            let te = ui.put(text_rect, egui::TextEdit::singleline(buf));
            te.request_focus();
            if te.lost_focus() {
                let commit = !ui.input(|i| i.key_pressed(egui::Key::Escape));
                let (rid, name) = ctx.renaming.take().unwrap();
                if commit && !name.trim().is_empty() {
                    ctx.actions.push(Action::Rename { id: rid, name: name.trim().to_string() });
                }
            }
        } else {
            painter.text(egui::pos2(rect.left() + 30.0, rect.center().y - 7.0), Align2::LEFT_CENTER, &flow.name, FontId::proportional(13.0), p.text);
            painter.text(
                egui::pos2(rect.left() + 30.0, rect.center().y + 8.0),
                Align2::LEFT_CENTER,
                format!("{} steps · {} wires", flow.nodes.len(), flow.edges.len()),
                FontId::proportional(11.0),
                p.dim,
            );
        }
        if resp.clicked() && !renaming {
            ctx.actions.push(Action::OpenFlow(flow.id.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Open").clicked() {
                ctx.actions.push(Action::OpenFlow(flow.id.clone()));
                ui.close_menu();
            }
            if ui.button("▶ Run").clicked() {
                ctx.actions.push(Action::OpenFlow(flow.id.clone()));
                ctx.actions.push(Action::RunFlow(flow.id.clone()));
                ui.close_menu();
            }
            if ui.button("Rename").clicked() {
                *ctx.renaming = Some((flow.id.clone(), flow.name.clone()));
                ui.close_menu();
            }
            if ui.button("Duplicate").clicked() {
                ctx.actions.push(Action::Duplicate { id: flow.id.clone() });
                ui.close_menu();
            }
            ui.separator();
            if ui.button(RichText::new("Delete").color(p.err)).clicked() {
                ctx.actions.push(Action::AskDelete { id: flow.id.clone(), name: flow.name.clone() });
                ui.close_menu();
            }
        });
    }
}
