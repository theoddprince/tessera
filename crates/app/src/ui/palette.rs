//! Ctrl+K command palette: fuzzy-jump to any request or action.

use egui::{Key, RichText};
use tessera_core::model::{Item, RequestKind};

use crate::app::{Action, SwiftApp};
use crate::ui::widgets::{self, dim, pal};

struct Entry {
    title: String,
    detail: String,
    badge: Option<String>,
    action: Action,
}

/// Subsequence fuzzy score; higher is better, None = no match.
fn fuzzy(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last = None;
    for qc in query.to_lowercase().chars() {
        let mut found = false;
        while ti < t.len() {
            if t[ti] == qc {
                score += if last == Some(ti.wrapping_sub(1)) { 6 } else { 1 };
                if ti == 0 || !t[ti - 1].is_alphanumeric() {
                    score += 4;
                }
                last = Some(ti);
                ti += 1;
                found = true;
                break;
            }
            ti += 1;
        }
        if !found {
            return None;
        }
    }
    Some(score - (t.len() as i32 / 12))
}

impl SwiftApp {
    fn palette_entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for c in &self.ws.collections {
            fn walk(out: &mut Vec<Entry>, items: &[Item], cid: &str, path: &str) {
                for i in items {
                    match i {
                        Item::Request(r) => out.push(Entry {
                            title: r.name.clone(),
                            detail: format!("{path} · {}", r.url),
                            badge: Some(r.badge().to_string()),
                            action: Action::OpenRequest { collection_id: cid.to_string(), request_id: r.id.clone() },
                        }),
                        Item::Folder(f) => walk(out, &f.items, cid, &format!("{path} / {}", f.name)),
                    }
                }
            }
            walk(&mut out, &c.items, &c.id, &c.name);
        }
        let mut act = |title: &str, detail: &str, action: Action| {
            out.push(Entry { title: title.into(), detail: detail.into(), badge: None, action });
        };
        act("New HTTP request", "Ctrl+T", Action::NewTab(RequestKind::Http));
        act("New GraphQL request", "", Action::NewTab(RequestKind::GraphQL));
        act("New WebSocket", "", Action::NewTab(RequestKind::WebSocket));
        act("New collection", "", Action::NewCollection);
        act("Import…", "Postman collections, environments, cURL", Action::OpenImport);
        act("Manage environments", "", Action::OpenEnvs);
        act("Settings", "Proxy, certificates, TLS, theme", Action::OpenSettings);
        act("Toggle dark / light theme", "", Action::ToggleTheme);
        act("Clear history", "", Action::ClearHistory);
        act("Switch environment: none", "", Action::SetEnv(None));
        for e in &self.ws.environments {
            out.push(Entry { title: format!("Switch environment: {}", e.name), detail: String::new(), badge: None, action: Action::SetEnv(Some(e.id.clone())) });
        }
        for c in &self.ws.collections {
            out.push(Entry {
                title: format!("Run collection: {}", c.name),
                detail: String::new(),
                badge: None,
                action: Action::OpenRunner { collection_id: c.id.clone(), folder_id: None },
            });
        }
        out
    }

    pub fn command_palette(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let q = self.palette.query.trim().to_string();
        let mut scored: Vec<(i32, Entry)> = self
            .palette_entries()
            .into_iter()
            .filter_map(|e| {
                let s = fuzzy(&q, &e.title).map(|s| s + 20).or_else(|| fuzzy(&q, &e.detail).map(|s| s - 10))?;
                Some((s, e))
            })
            .collect();
        if !q.is_empty() {
            scored.sort_by_key(|e| std::cmp::Reverse(e.0));
        }
        scored.truncate(60);
        let n = scored.len();
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        if down && n > 0 {
            self.palette.selected = (self.palette.selected + 1) % n;
        }
        if up && n > 0 {
            self.palette.selected = (self.palette.selected + n - 1) % n;
        }
        self.palette.selected = self.palette.selected.min(n.saturating_sub(1));
        let mut chosen: Option<Action> = None;
        if enter {
            chosen = scored.get(self.palette.selected).map(|(_, e)| e.action.clone());
        }
        let mut close = esc;
        let dark = self.ws.settings.dark_mode;

        egui::Area::new("palette-bg".into()).order(egui::Order::Middle).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| {
            let screen = ctx.screen_rect();
            let r = ui.allocate_rect(screen, egui::Sense::click());
            ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(90));
            if r.clicked() {
                close = true;
            }
        });
        egui::Area::new("palette".into()).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, 80.0]).show(ctx, |ui| {
            let p = pal(ui);
            egui::Frame::popup(ui.style()).fill(p.panel).corner_radius(10).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(600.0);
                let r = ui.add(egui::TextEdit::singleline(&mut self.palette.query).hint_text("Search requests, environments, actions…").desired_width(f32::INFINITY).font(egui::FontId::proportional(16.0)));
                if self.palette.focus_pending {
                    r.request_focus();
                    self.palette.focus_pending = false;
                }
                if r.changed() {
                    self.palette.selected = 0;
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    if scored.is_empty() {
                        ui.label(dim(ui, "No matches"));
                    }
                    for (i, (_, e)) in scored.iter().enumerate() {
                        let selected = i == self.palette.selected;
                        let frame = egui::Frame::new().fill(if selected { p.accent.gamma_multiply(0.2) } else { egui::Color32::TRANSPARENT }).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 5));
                        let resp = frame
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    match &e.badge {
                                        Some(b) => widgets::method_badge(ui, b),
                                        None => {
                                            ui.label(RichText::new("›").color(p.accent));
                                        }
                                    }
                                    ui.label(RichText::new(&e.title).color(p.text));
                                    if !e.detail.is_empty() {
                                        let mut d = e.detail.clone();
                                        if d.chars().count() > 60 {
                                            d = d.chars().take(59).collect::<String>() + "…";
                                        }
                                        ui.label(dim(ui, d).size(11.5));
                                    }
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if selected && (up || down) {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            chosen = Some(e.action.clone());
                        }
                        if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                            self.palette.selected = i;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(dim(ui, "↑↓ navigate · Enter open · Esc close").size(11.0));
                let _ = dark;
            });
        });
        if let Some(a) = chosen {
            self.actions.push(a);
            close = true;
        }
        if close {
            self.palette.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fuzzy;

    #[test]
    fn fuzzy_ranks_prefix_and_contiguous_higher() {
        let a = fuzzy("get", "Get users").unwrap();
        let b = fuzzy("get", "Upload a great token").unwrap();
        assert!(a > b);
        assert!(fuzzy("xyz", "abc").is_none());
    }
}
