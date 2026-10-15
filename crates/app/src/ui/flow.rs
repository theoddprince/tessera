//! Flow canvas: boxes for requests, wires that carry values between them.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::Value;
use tessera_core::flow::{Flow, FlowNode, MapSource, Mapping, NodeResult, NodeStatus};
use tessera_core::model::{new_id, AuthKind, Collection, Item, Request};

use crate::app::Action;
use crate::theme;
use crate::ui::sidebar::DragItem;
use crate::ui::widgets::{self, dim, pal};

const NODE_W: f32 = 236.0;
const NODE_H: f32 = 70.0;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Selection {
    #[default]
    None,
    Node(String),
    Edge(String),
}

#[derive(Default)]
pub struct FlowRun {
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
    pub results: HashMap<String, NodeResult>,
    pub started: Option<Instant>,
    pub elapsed_ms: f64,
    pub flow_vars: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct FlowView {
    pub pan: Vec2,
    pub selected: Selection,
    pub connecting: Option<String>,
    pub run: FlowRun,
}

fn find<'a>(cols: &'a [Collection], n: &FlowNode) -> Option<(&'a Collection, &'a Request)> {
    let c = cols.iter().find(|c| c.id == n.collection_id)?;
    Some((c, c.find_request(&n.request_id)?))
}

fn bezier_point(p: [Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    Pos2::new(
        p.iter().zip(w).map(|(p, w)| p.x * w).sum(),
        p.iter().zip(w).map(|(p, w)| p.y * w).sum(),
    )
}

fn wire(a: Pos2, b: Pos2) -> [Pos2; 4] {
    let dx = ((b.x - a.x).abs() * 0.5).max(50.0);
    [a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b]
}

fn status_color(r: Option<&NodeResult>, p: &theme::Palette) -> Option<Color32> {
    r.map(|r| match r.status {
        NodeStatus::Running => p.warn,
        NodeStatus::Passed => p.ok,
        NodeStatus::Failed => p.err,
        NodeStatus::Skipped => p.dim,
    })
}

fn mapping_summary(m: &[Mapping]) -> String {
    let named: Vec<String> = m
        .iter()
        .filter(|m| !m.variable.trim().is_empty())
        .map(|m| {
            let from = match m.source {
                MapSource::Body => m.path.trim().to_string(),
                MapSource::Header => format!("header {}", m.path.trim()),
                MapSource::Status => "status".into(),
            };
            format!("{from} → {{{{{}}}}}", m.variable.trim())
        })
        .collect();
    if named.is_empty() {
        "+ pass a value".into()
    } else if named.len() > 2 {
        format!("{}, … (+{})", named[..2].join(", "), named.len() - 2)
    } else {
        named.join(", ")
    }
}

/// Adds nodes for a dropped/picked sidebar item (a request, or every request in a folder).
fn add_item(flow: &mut Flow, cols: &[Collection], collection_id: &str, item_id: &str, at: Vec2) -> Option<String> {
    let c = cols.iter().find(|c| c.id == collection_id)?;
    let mut reqs: Vec<&Request> = Vec::new();
    if let Some(r) = c.find_request(item_id) {
        reqs.push(r);
    } else {
        fn walk<'a>(items: &'a [Item], target: &str, inside: bool, out: &mut Vec<&'a Request>) {
            for i in items {
                match i {
                    Item::Request(r) if inside => out.push(r),
                    Item::Folder(f) => walk(&f.items, target, inside || f.id == target, out),
                    _ => {}
                }
            }
        }
        walk(&c.items, item_id, false, &mut reqs);
    }
    let mut last = None;
    for (i, r) in reqs.iter().enumerate() {
        let id = new_id();
        flow.nodes.push(FlowNode {
            id: id.clone(),
            collection_id: c.id.clone(),
            request_id: r.id.clone(),
            pos: [at.x + i as f32 * (NODE_W + 70.0), at.y],
        });
        last = Some(id);
    }
    last
}

/// Returns true when the flow was modified.
pub fn show(ui: &mut Ui, flow: &mut Flow, view: &mut FlowView, cols: &[Collection], actions: &mut Vec<Action>) -> bool {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    let mut changed = false;

    // ---------------------------------------------------------------- toolbar
    ui.horizontal(|ui| {
        ui.label(RichText::new("FLOW").strong().size(11.0).color(theme::method_color("WS", dark)));
        changed |= ui.add(egui::TextEdit::singleline(&mut flow.name).font(egui::TextStyle::Heading).desired_width(260.0).frame(false)).changed();
        ui.separator();
        if view.run.running {
            if ui.add(egui::Button::new(RichText::new("■ Stop").color(Color32::WHITE)).fill(p.err).min_size(egui::vec2(90.0, 28.0))).clicked() {
                actions.push(Action::StopFlow(flow.id.clone()));
            }
            ui.spinner();
        } else {
            let btn = egui::Button::new(RichText::new("▶ Run flow").color(Color32::WHITE).strong()).fill(p.accent).min_size(egui::vec2(110.0, 28.0));
            if ui.add_enabled(!flow.nodes.is_empty(), btn).on_hover_text("Ctrl+Enter").clicked() {
                actions.push(Action::RunFlow(flow.id.clone()));
            }
        }
        ui.menu_button("➕ Add request", |ui| {
            ui.set_max_height(420.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                if cols.is_empty() {
                    ui.label(dim(ui, "No collections yet"));
                }
                for c in cols {
                    ui.label(RichText::new(&c.name).strong());
                    for r in c.all_requests() {
                        let resp = ui.horizontal(|ui| {
                            widgets::method_badge(ui, r.badge());
                            ui.add(egui::Button::new(&r.name).frame(false))
                        });
                        if resp.inner.clicked() {
                            let n = flow.nodes.len() as f32;
                            let at = Vec2::new(40.0 + (n % 4.0) * (NODE_W + 70.0), 40.0 + (n / 4.0).floor() * (NODE_H + 60.0)) - view.pan;
                            if let Some(id) = add_item(flow, cols, &c.id, &r.id, at) {
                                view.selected = Selection::Node(id);
                                changed = true;
                            }
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                }
            });
        });
        if ui.button("⌖ Center").on_hover_text("Bring all steps into view").clicked() {
            view.pan = Vec2::ZERO;
            if let Some(min) = flow.nodes.iter().map(|n| Vec2::new(n.pos[0], n.pos[1])).reduce(|a, b| a.min(b)) {
                view.pan = Vec2::new(30.0, 30.0) - min;
            }
        }
        let results = &view.run.results;
        if !results.is_empty() {
            let count = |s| results.values().filter(|r| r.status == s).count();
            ui.separator();
            widgets::chip(ui, format!("{} passed", count(NodeStatus::Passed)), p.ok);
            if count(NodeStatus::Failed) > 0 {
                widgets::chip(ui, format!("{} failed", count(NodeStatus::Failed)), p.err);
            }
            if count(NodeStatus::Skipped) > 0 {
                widgets::chip(ui, format!("{} skipped", count(NodeStatus::Skipped)), p.dim);
            }
            let ms = if view.run.running { view.run.started.map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0) } else { view.run.elapsed_ms };
            ui.label(dim(ui, widgets::human_time(ms)));
        }
    });
    ui.add_space(6.0);

    // ---------------------------------------------------------------- inspector
    egui::SidePanel::right(egui::Id::new(("flow-inspector", &flow.id)))
        .resizable(true)
        .default_width(360.0)
        .width_range(280.0..=600.0)
        .frame(egui::Frame::new().fill(p.surface).inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 0 }))
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                changed |= inspector(ui, flow, view, cols, actions);
            });
        });

    // ---------------------------------------------------------------- canvas
    let (canvas, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(canvas);
    painter.rect_filled(canvas, 8.0, if dark { p.bg } else { p.surface_hi });
    let grid = 24.0;
    let offset = Vec2::new(view.pan.x.rem_euclid(grid), view.pan.y.rem_euclid(grid));
    let dot = p.border;
    let mut y = canvas.top() + offset.y;
    while y < canvas.bottom() {
        let mut x = canvas.left() + offset.x;
        while x < canvas.right() {
            painter.circle_filled(Pos2::new(x, y), 1.0, dot);
            x += grid;
        }
        y += grid;
    }
    let origin = canvas.min.to_vec2() + view.pan;
    let node_rect = |n: &FlowNode| Rect::from_min_size(Pos2::new(n.pos[0], n.pos[1]) + origin, Vec2::new(NODE_W, NODE_H));

    // Pan by dragging empty canvas or scrolling.
    if bg.dragged() && view.connecting.is_none() {
        view.pan += bg.drag_delta();
    }
    if bg.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        view.pan += scroll;
    }
    if bg.clicked() {
        view.selected = Selection::None;
    }

    // Drop requests/folders from the sidebar.
    if let Some(drag) = bg.dnd_hover_payload::<DragItem>() {
        let _ = drag;
        painter.rect_stroke(canvas.shrink(2.0), 8.0, Stroke::new(2.0_f32, p.accent), egui::StrokeKind::Inside);
    }
    if let Some(drag) = bg.dnd_release_payload::<DragItem>() {
        let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(canvas.center()) - origin - Vec2::new(NODE_W / 2.0, NODE_H / 2.0);
        if let Some(id) = add_item(flow, cols, &drag.collection_id, &drag.item_id, at.to_vec2()) {
            view.selected = Selection::Node(id);
            changed = true;
        }
    }

    if flow.nodes.is_empty() {
        painter.text(canvas.center() - Vec2::new(0.0, 14.0), Align2::CENTER_CENTER, "Drag requests from the sidebar onto this canvas", FontId::proportional(15.0), p.text);
        painter.text(canvas.center() + Vec2::new(0.0, 12.0), Align2::CENTER_CENTER, "or use ➕ Add request — then drag from a step's ● to the next step to connect them", FontId::proportional(12.5), p.dim);
    }

    // Edges (under nodes).
    let mut clicked_edge = None;
    for e in &flow.edges {
        let (Some(a), Some(b)) = (flow.node(&e.from), flow.node(&e.to)) else { continue };
        let (ra, rb) = (node_rect(a), node_rect(b));
        let pts = wire(ra.right_center(), rb.left_center());
        let selected = view.selected == Selection::Edge(e.id.clone());
        let color = match view.run.results.get(&e.from).map(|r| r.status) {
            Some(NodeStatus::Passed) => p.ok,
            Some(NodeStatus::Failed) => p.err,
            _ => p.accent.gamma_multiply(0.8),
        };
        let width: f32 = if selected { 3.0 } else { 2.0 };
        painter.add(CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(width, color)));
        let tip = pts[3];
        painter.add(egui::Shape::convex_polygon(vec![tip, tip + Vec2::new(-9.0, -5.0), tip + Vec2::new(-9.0, 5.0)], color, Stroke::NONE));

        // Clickable label in the middle of the wire.
        let mid = bezier_point(pts, 0.5);
        let text = mapping_summary(&e.mappings);
        let empty = e.mappings.iter().all(|m| m.variable.trim().is_empty());
        let galley = ui.fonts(|f| f.layout_no_wrap(text, FontId::proportional(11.5), if empty { p.dim } else { p.text }));
        let label_rect = Rect::from_center_size(mid, galley.size() + Vec2::new(14.0, 8.0));
        let resp = ui.interact(label_rect, ui.id().with(("edge", &e.id)), Sense::click());
        let fill = if selected || resp.hovered() { p.surface_hi } else { p.surface };
        painter.rect(label_rect, 10.0, fill, Stroke::new(1.0_f32, if selected { p.accent } else { color.gamma_multiply(0.7) }), egui::StrokeKind::Inside);
        painter.galley(label_rect.min + Vec2::new(7.0, 4.0), galley, p.text);
        if resp.clicked() {
            clicked_edge = Some(e.id.clone());
        }
        resp.on_hover_text("Click to choose which values pass along this wire");
    }
    if let Some(id) = clicked_edge {
        view.selected = Selection::Edge(id);
    }

    // Nodes.
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut remove: Option<String> = None;
    let mut start_connect: Option<String> = None;
    for n in flow.nodes.iter_mut() {
        let rect = Rect::from_min_size(Pos2::new(n.pos[0], n.pos[1]) + origin, Vec2::new(NODE_W, NODE_H));
        if !canvas.intersects(rect) {
            continue;
        }
        let resp = ui.interact(rect, ui.id().with(("node", &n.id)), Sense::click_and_drag());
        if resp.dragged() && view.connecting.is_none() {
            n.pos[0] += resp.drag_delta().x;
            n.pos[1] += resp.drag_delta().y;
            changed = true;
        }
        if resp.clicked() || resp.drag_started() {
            view.selected = Selection::Node(n.id.clone());
        }
        let info = find(cols, n);
        if resp.double_clicked() {
            actions.push(Action::OpenRequest { collection_id: n.collection_id.clone(), request_id: n.request_id.clone() });
        }
        let node_id = n.id.clone();
        resp.context_menu(|ui| {
            if ui.button("Open request").clicked() {
                actions.push(Action::OpenRequest { collection_id: n.collection_id.clone(), request_id: n.request_id.clone() });
                ui.close_menu();
            }
            if ui.button(RichText::new("Remove from flow").color(p.err)).clicked() {
                remove = Some(node_id.clone());
                ui.close_menu();
            }
        });

        let result = view.run.results.get(&n.id);
        let selected = view.selected == Selection::Node(n.id.clone());
        let border = if selected { p.accent } else { status_color(result, &p).unwrap_or(p.border) };
        let painter = ui.painter_at(canvas);
        painter.rect(rect.translate(Vec2::new(0.0, 3.0)), 10.0, Color32::from_black_alpha(if dark { 70 } else { 20 }), Stroke::NONE, egui::StrokeKind::Inside);
        painter.rect(rect, 10.0, p.panel, Stroke::new(if selected { 2.0_f32 } else { 1.5 }, border), egui::StrokeKind::Inside);
        if let Some(c) = status_color(result, &p) {
            painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(5.0, NODE_H)).shrink2(Vec2::new(0.0, 8.0)).translate(Vec2::new(3.0, 0.0)), 2.0, c);
        }
        let left = rect.left() + 16.0;
        match info {
            Some((c, r)) => {
                painter.text(Pos2::new(left, rect.top() + 18.0), Align2::LEFT_CENTER, short_method(r.badge()), FontId::monospace(11.0), theme::method_color(r.badge(), dark));
                let name = ui.fonts(|f| {
                    let mut job = egui::text::LayoutJob::simple_singleline(r.name.clone(), FontId::proportional(13.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(NODE_W - 70.0);
                    f.layout_job(job)
                });
                painter.galley(Pos2::new(left + 42.0, rect.top() + 18.0 - name.size().y / 2.0), name, p.text);
                let (line, color) = match result {
                    None => (c.name.clone(), p.dim),
                    Some(res) => match res.status {
                        NodeStatus::Running => ("running…".to_string(), p.warn),
                        NodeStatus::Passed => (format!("✔ {} · {}", res.http_status.unwrap_or(0), widgets::human_time(res.time_ms)), p.ok),
                        NodeStatus::Failed => (format!("✖ {}", res.error.clone().unwrap_or_default()), p.err),
                        NodeStatus::Skipped => ("⏭ skipped".to_string(), p.dim),
                    },
                };
                let sub = ui.fonts(|f| {
                    let mut job = egui::text::LayoutJob::simple_singleline(line, FontId::proportional(12.0), color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(NODE_W - 30.0);
                    f.layout_job(job)
                });
                painter.galley(Pos2::new(left, rect.top() + 46.0 - sub.size().y / 2.0), sub, color);
            }
            None => {
                painter.text(Pos2::new(left, rect.center().y), Align2::LEFT_CENTER, "⚠ request was deleted", FontId::proportional(13.0), p.err);
            }
        }
        if result.is_some_and(|r| r.status == NodeStatus::Running) {
            ui.ctx().request_repaint();
        }

        // Ports.
        painter.circle(rect.left_center(), 5.0, p.panel, Stroke::new(2.0_f32, p.dim));
        let out = rect.right_center();
        let port_rect = Rect::from_center_size(out, Vec2::splat(20.0));
        let port = ui.interact(port_rect, ui.id().with(("port", &n.id)), Sense::drag());
        let hot = port.hovered() || view.connecting.as_deref() == Some(n.id.as_str());
        painter.circle(out, if hot { 7.0 } else { 6.0 }, if hot { p.accent } else { p.panel }, Stroke::new(2.0_f32, p.accent));
        let port = port.on_hover_text("Drag to the next step to connect");
        if port.drag_started() {
            start_connect = Some(n.id.clone());
        }
    }
    if let Some(id) = start_connect {
        view.connecting = Some(id);
    }

    // Wire being drawn.
    if let Some(from) = view.connecting.clone() {
        if let (Some(a), Some(ptr)) = (flow.node(&from), pointer) {
            let start = node_rect(a).right_center();
            painter.add(CubicBezierShape::from_points_stroke(wire(start, ptr), false, Color32::TRANSPARENT, Stroke::new(2.0_f32, p.accent)));
            if let Some(target) = flow.nodes.iter().find(|n| n.id != from && node_rect(n).contains(ptr)) {
                painter.rect_stroke(node_rect(target).expand(3.0), 12.0, Stroke::new(2.0_f32, p.accent), egui::StrokeKind::Outside);
            }
        }
        if ui.input(|i| i.pointer.any_released()) {
            let target = pointer.and_then(|ptr| flow.nodes.iter().find(|n| n.id != from && node_rect(n).contains(ptr)).map(|n| n.id.clone()));
            if let Some(to) = target {
                match flow.connect(&from, &to) {
                    Some(edge) => {
                        view.selected = Selection::Edge(edge);
                        changed = true;
                    }
                    None => actions.push(Action::Toast("Those steps are already connected, or the wire would create a loop".into(), true)),
                }
            }
            view.connecting = None;
        }
    }

    // Delete key removes the selection (when not typing).
    if !ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        match view.selected.clone() {
            Selection::Node(id) => remove = Some(id),
            Selection::Edge(id) => {
                flow.edges.retain(|e| e.id != id);
                view.selected = Selection::None;
                changed = true;
            }
            Selection::None => {}
        }
    }
    if let Some(id) = remove {
        flow.remove_node(&id);
        view.run.results.remove(&id);
        view.selected = Selection::None;
        changed = true;
    }
    changed
}

fn short_method(m: &str) -> &str {
    match m {
        "DELETE" => "DEL",
        "OPTIONS" => "OPT",
        "PATCH" => "PTCH",
        m => m,
    }
}

fn request_label(cols: &[Collection], flow: &Flow, node_id: &str) -> String {
    flow.node(node_id).and_then(|n| find(cols, n)).map(|(_, r)| r.name.clone()).unwrap_or_else(|| "?".into())
}

fn inspector(ui: &mut Ui, flow: &mut Flow, view: &mut FlowView, cols: &[Collection], actions: &mut Vec<Action>) -> bool {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    let mut changed = false;
    match view.selected.clone() {
        Selection::None => {
            ui.label(RichText::new("How flows work").strong().size(15.0));
            ui.add_space(4.0);
            for (n, text) in [
                ("1", "Drag requests from the sidebar onto the canvas (or ➕ Add request)."),
                ("2", "Drag from a step's ● on the right to the next step to connect them."),
                ("3", "Click the wire's label and choose what to pass — e.g. access_token → {{token}}. After one run you can just click the field in the response."),
                ("4", "Use {{token}} in the next request (e.g. Auth → Bearer {{token}}) and press ▶ Run flow."),
            ] {
                ui.horizontal_top(|ui| {
                    widgets::chip(ui, n, p.accent);
                    ui.add(egui::Label::new(text).wrap());
                });
                ui.add_space(2.0);
            }
            ui.add_space(6.0);
            ui.label(dim(ui, "Steps run in wire order. If a step fails (error, HTTP ≥ 400, failed tests, or a value can't be found) the steps after it are skipped. Flows use the saved version of each request.").size(11.5));
            if !view.run.flow_vars.is_empty() {
                ui.separator();
                ui.label(RichText::new("Values from the last run").strong());
                egui::Grid::new("flow-vars").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, v) in &view.run.flow_vars {
                        ui.label(RichText::new(format!("{{{{{k}}}}}")).monospace().color(p.json_key));
                        ui.add(egui::Label::new(RichText::new(v).monospace()).truncate()).on_hover_text(v);
                        ui.end_row();
                    }
                });
            }
        }
        Selection::Node(id) => {
            let Some(node) = flow.node(&id).cloned() else {
                view.selected = Selection::None;
                return false;
            };
            match find(cols, &node) {
                Some((c, r)) => {
                    ui.horizontal(|ui| {
                        ui.label(widgets::method_text(r.badge(), dark, 12.0));
                        ui.label(RichText::new(&r.name).strong().size(15.0));
                    });
                    ui.label(dim(ui, format!("{} · {}", c.name, r.url)).size(11.5));
                }
                None => {
                    ui.label(RichText::new("This request was deleted").color(p.err));
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Open request").clicked() {
                    actions.push(Action::OpenRequest { collection_id: node.collection_id.clone(), request_id: node.request_id.clone() });
                }
                if ui.button(RichText::new("Remove").color(p.err)).clicked() {
                    flow.remove_node(&id);
                    view.run.results.remove(&id);
                    view.selected = Selection::None;
                    changed = true;
                }
            });
            ui.separator();
            match view.run.results.get(&id) {
                None => {
                    ui.label(dim(ui, "Not run yet. Press ▶ Run flow."));
                }
                Some(res) => node_result(ui, res),
            }
        }
        Selection::Edge(id) => {
            let Some(edge_idx) = flow.edges.iter().position(|e| e.id == id) else {
                view.selected = Selection::None;
                return false;
            };
            let (from, to) = (flow.edges[edge_idx].from.clone(), flow.edges[edge_idx].to.clone());
            let (from_name, to_name) = (request_label(cols, flow, &from), request_label(cols, flow, &to));
            ui.label(RichText::new("Pass values").strong().size(15.0));
            ui.label(dim(ui, format!("from “{from_name}” → to “{to_name}”")).size(12.0));
            ui.add_space(6.0);

            let edge = &mut flow.edges[edge_idx];
            let mut remove_row = None;
            for (i, m) in edge.mappings.iter_mut().enumerate() {
                egui::Frame::new().fill(p.panel).corner_radius(8).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("map-src", &id, i)).width(110.0).selected_text(m.source.label()).show_ui(ui, |ui| {
                            for s in MapSource::ALL {
                                changed |= ui.selectable_value(&mut m.source, s, s.label()).changed();
                            }
                        });
                        if m.source != MapSource::Status {
                            let hint = if m.source == MapSource::Body { "access_token  or  data.items[0].id" } else { "X-Auth-Token" };
                            changed |= ui.add(egui::TextEdit::singleline(&mut m.path).hint_text(hint).desired_width(ui.available_width() - 30.0).font(egui::TextStyle::Monospace)).changed();
                        }
                        if ui.add(egui::Button::new(RichText::new("✕").color(p.dim)).frame(false)).clicked() {
                            remove_row = Some(i);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("→ {{").monospace().color(p.dim));
                        changed |= ui.add(egui::TextEdit::singleline(&mut m.variable).hint_text("token").desired_width(140.0).font(egui::TextStyle::Monospace)).changed();
                        ui.label(RichText::new("}}").monospace().color(p.dim));
                        changed |= ui.checkbox(&mut m.save_to_env, "save to env").on_hover_text("Also store it in the active environment so you can send the next request manually too").changed();
                    });
                });
                ui.add_space(4.0);
            }
            if let Some(i) = remove_row {
                edge.mappings.remove(i);
                changed = true;
            }
            let mut delete_wire = false;
            ui.horizontal(|ui| {
                if ui.button("➕ Add value").clicked() {
                    edge.mappings.push(Mapping { save_to_env: true, ..Default::default() });
                    changed = true;
                }
                delete_wire = ui.button(RichText::new("Delete wire").color(p.err)).clicked();
            });
            if delete_wire {
                flow.edges.remove(edge_idx);
                view.selected = Selection::None;
                return true;
            }
            let Some(edge) = flow.edges.iter_mut().find(|e| e.id == id) else { return changed };

            // One-click "use it as the bearer token" for the classic login → data case.
            if let Some(var) = edge.mappings.iter().map(|m| m.variable.trim()).find(|v| !v.is_empty()) {
                if let Some((c, r)) = flow_node_request(cols, &to, &flow.nodes) {
                    // Look at the auth the request really sends (it may inherit it from its folder/collection).
                    let token = format!("{{{{{var}}}}}");
                    let auth = c.effective_auth(r);
                    let uses = r.url.contains(&token)
                        || r.headers.iter().any(|h| h.value.contains(&token))
                        || r.body.raw.contains(&token)
                        || r.body.graphql_variables.contains(&token)
                        || [&auth.bearer_token, &auth.basic_user, &auth.basic_pass, &auth.apikey_value, &auth.oauth2.access_token]
                            .iter()
                            .any(|v| v.contains(&token));
                    ui.add_space(6.0);
                    if uses {
                        ui.label(RichText::new(format!("✔ “{to_name}” uses {{{{{var}}}}}")).color(p.ok).size(12.0));
                    } else {
                        ui.label(RichText::new(format!("“{to_name}” doesn't use {{{{{var}}}}} yet.")).color(p.warn).size(12.0));
                        ui.horizontal(|ui| {
                            if ui.button(format!("Use as Bearer token in “{to_name}”")).clicked() {
                                actions.push(Action::SetBearer { collection_id: c.id.clone(), request_id: r.id.clone(), variable: var.to_string() });
                            }
                        });
                        if r.auth.kind != AuthKind::Inherit && r.auth.kind != AuthKind::None && r.auth.kind != AuthKind::Bearer {
                            ui.label(dim(ui, "(this will replace its current auth)").size(11.0));
                        }
                    }
                }
            }

            ui.separator();
            ui.label(RichText::new(format!("Pick from “{from_name}” response")).strong());
            match view.run.results.get(&from) {
                Some(res) if res.json.is_some() || res.response.is_some() => {
                    ui.label(dim(ui, "Click a value to pass it along this wire.").size(11.5));
                    let mut picked: Option<Mapping> = None;
                    if let Some(json) = &res.json {
                        egui::Frame::new().fill(p.panel).corner_radius(8).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.spacing_mut().interact_size.y = 20.0;
                            picker(ui, None, json, "", 0, &mut picked);
                        });
                    }
                    if let Some(resp) = &res.response {
                        egui::CollapsingHeader::new("Headers").id_salt(("pick-h", &id)).show(ui, |ui| {
                            for (k, v) in &resp.headers {
                                let r = ui.add(egui::Label::new(RichText::new(format!("{k}: {v}")).monospace().size(11.5)).truncate().sense(Sense::click()));
                                if r.on_hover_text("Click to pass this header").clicked() {
                                    picked = Some(Mapping { source: MapSource::Header, path: k.clone(), variable: k.replace('-', "_").to_lowercase(), save_to_env: true });
                                }
                            }
                        });
                    }
                    if let Some(m) = picked {
                        // Fill an empty row if there is one, otherwise add a new one.
                        match edge.mappings.iter_mut().find(|x| x.path.trim().is_empty() && x.variable.trim().is_empty()) {
                            Some(slot) => *slot = m,
                            None => edge.mappings.push(m),
                        }
                        changed = true;
                    }
                }
                _ => {
                    ui.label(dim(ui, "Run the flow once and the response appears here — then just click the field you need (e.g. access_token).").size(11.5));
                }
            }
        }
    }
    changed
}

fn flow_node_request<'a>(cols: &'a [Collection], node_id: &str, nodes: &[FlowNode]) -> Option<(&'a Collection, &'a Request)> {
    find(cols, nodes.iter().find(|n| n.id == node_id)?)
}

fn node_result(ui: &mut Ui, res: &NodeResult) {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    ui.horizontal(|ui| {
        match res.status {
            NodeStatus::Running => {
                ui.spinner();
                ui.label("Running…");
            }
            NodeStatus::Passed => {
                widgets::chip(ui, "Passed", p.ok);
            }
            NodeStatus::Failed => {
                widgets::chip(ui, "Failed", p.err);
            }
            NodeStatus::Skipped => {
                widgets::chip(ui, "Skipped", p.dim);
            }
        }
        if let Some(s) = res.http_status {
            ui.label(RichText::new(s.to_string()).color(theme::status_color(s, dark)));
            ui.label(dim(ui, widgets::human_time(res.time_ms)));
        }
    });
    if let Some(e) = &res.error {
        ui.add(egui::Label::new(RichText::new(e).color(if res.status == NodeStatus::Skipped { p.dim } else { p.err })).wrap());
    }
    if !res.extracted.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("Passed along").strong());
        for (k, v) in &res.extracted {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{{{{{k}}}}}")).monospace().color(p.json_key));
                ui.add(egui::Label::new(RichText::new(v).monospace()).truncate()).on_hover_text(v);
            });
        }
    }
    if !res.tests.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("Tests").strong());
        for t in &res.tests {
            ui.horizontal(|ui| {
                ui.label(RichText::new(if t.passed { "✔" } else { "✖" }).color(if t.passed { p.ok } else { p.err }));
                ui.label(&t.name);
            });
        }
    }
    if let Some(resp) = &res.response {
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new("Response body").strong()).default_open(true).show(ui, |ui| {
            let text = match &res.json {
                Some(j) => serde_json::to_string_pretty(j).unwrap_or_default(),
                None => resp.text(),
            };
            let shown: String = text.lines().take(300).collect::<Vec<_>>().join("\n");
            egui::ScrollArea::both().max_height(360.0).show(ui, |ui| {
                let mut s = shown.as_str();
                ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_width(f32::INFINITY));
            });
        });
    }
}

/// JSON tree where clicking a leaf (or a node's key) yields a mapping for that path.
fn picker(ui: &mut Ui, key: Option<&str>, v: &Value, path: &str, depth: usize, out: &mut Option<Mapping>) {
    let p = pal(ui);
    let var_name = |path: &str| -> String {
        let last = path.rsplit(['.', '[']).next().unwrap_or(path).trim_end_matches(']').trim_matches('"');
        if last.is_empty() || last.chars().all(|c| c.is_ascii_digit()) {
            "value".into()
        } else {
            last.to_string()
        }
    };
    match v {
        Value::Object(map) => {
            let id = ui.make_persistent_id(("pick", path));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < 2)
                .show_header(ui, |ui| {
                    ui.label(RichText::new(format!("{}{{{}}}", key.map(|k| format!("{k}: ")).unwrap_or_default(), map.len())).monospace().color(p.dim));
                })
                .body(|ui| {
                    for (k, child) in map.iter().take(200) {
                        let child_path = if path.is_empty() {
                            k.clone()
                        } else if k.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            format!("{path}.{k}")
                        } else {
                            format!("{path}[\"{k}\"]")
                        };
                        picker(ui, Some(k), child, &child_path, depth + 1, out);
                    }
                });
        }
        Value::Array(arr) => {
            let id = ui.make_persistent_id(("pick", path));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < 1)
                .show_header(ui, |ui| {
                    ui.label(RichText::new(format!("{}[{}]", key.map(|k| format!("{k}: ")).unwrap_or_default(), arr.len())).monospace().color(p.dim));
                })
                .body(|ui| {
                    for (i, child) in arr.iter().enumerate().take(50) {
                        picker(ui, Some(&i.to_string()), child, &format!("{path}[{i}]"), depth + 1, out);
                    }
                });
        }
        leaf => {
            let (text, color) = match leaf {
                Value::String(s) => (format!("{s:?}"), p.json_str),
                Value::Number(n) => (n.to_string(), p.json_num),
                Value::Bool(b) => (b.to_string(), p.json_kw),
                _ => ("null".into(), p.json_kw),
            };
            let mut job = egui::text::LayoutJob::default();
            let font = FontId::monospace(12.5);
            job.append(&format!("{}: ", key.unwrap_or("value")), 18.0, egui::TextFormat::simple(font.clone(), p.json_key));
            job.append(&text, 0.0, egui::TextFormat::simple(font, color));
            let resp = ui.add(egui::Label::new(job).truncate().selectable(false).sense(Sense::click()));
            if resp.hovered() {
                ui.painter().rect_filled(resp.rect, 4.0, p.accent.gamma_multiply(0.12));
            }
            if resp.on_hover_text(format!("Pass {path} along this wire")).clicked() {
                *out = Some(Mapping { source: MapSource::Body, path: path.to_string(), variable: var_name(path), save_to_env: true });
            }
        }
    }
}

impl FlowView {
    pub fn reset_run(&mut self) {
        self.run.results.clear();
        self.run.flow_vars.clear();
        self.run.started = Some(Instant::now());
        self.run.running = true;
        self.run.cancel = Arc::new(AtomicBool::new(false));
    }
}
