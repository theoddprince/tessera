//! Modal-ish windows.

use egui::{RichText, Ui};
use tessera_core::curl::{snippet, SnippetLang};
use tessera_core::model::{CertKind, ClientCert, Environment, Item};

use crate::app::{Action, SwiftApp};
use crate::bridge::AuthOwner;
use crate::state::ItemSection;
use crate::theme;
use crate::ui::request::{auth_editor, scripts_editor};
use crate::ui::widgets::{self, code_editor, dim, form_row, kv_table, pal, KvOptions, Lang};

fn window(title: &str) -> egui::Window<'_> {
    egui::Window::new(title).collapsible(false).resizable(true).pivot(egui::Align2::CENTER_CENTER).default_pos_center()
}

trait CenterExt {
    fn default_pos_center(self) -> Self;
}
impl CenterExt for egui::Window<'_> {
    fn default_pos_center(self) -> Self {
        self.anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
    }
}

impl SwiftApp {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        self.env_window(ctx);
        self.settings_window(ctx);
        self.import_window(ctx);
        self.snippet_window(ctx);
        self.item_settings_window(ctx);
        self.save_as_window(ctx);
        self.confirm_delete_window(ctx);
        self.runner_window(ctx);
    }

    fn env_window(&mut self, ctx: &egui::Context) {
        if !self.show_envs {
            return;
        }
        let mut open = true;
        let mut changed = false;
        let mut actions = Vec::new();
        window("Environments").open(&mut open).default_size([820.0, 480.0]).show(ctx, |ui| {
            let p = pal(ui);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    let globals_sel = self.env_selected.is_none();
                    if ui.selectable_label(globals_sel, "🌐 Globals").clicked() {
                        self.env_selected = None;
                    }
                    ui.separator();
                    for e in &self.ws.environments {
                        let active = self.ws.active_env.as_ref() == Some(&e.id);
                        let label = if active { format!("● {}", e.name) } else { e.name.clone() };
                        let r = ui.selectable_label(self.env_selected.as_ref() == Some(&e.id), RichText::new(label).color(if active { p.ok } else { p.text }));
                        if r.clicked() {
                            self.env_selected = Some(e.id.clone());
                        }
                    }
                    ui.add_space(6.0);
                    if ui.button("➕ New environment").clicked() {
                        let e = Environment::new(format!("Environment {}", self.ws.environments.len() + 1));
                        self.env_selected = Some(e.id.clone());
                        self.ws.environments.push(e);
                        changed = true;
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    match self.env_selected.clone() {
                        None => {
                            ui.label(RichText::new("Globals").strong().size(15.0));
                            ui.label(dim(ui, "Available in every request, lowest precedence."));
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical().id_salt("globals").show(ui, |ui| {
                                changed |= kv_table(ui, "globals-kv", &mut self.ws.globals, KvOptions { secrets: true, key_hint: "Variable", ..Default::default() });
                            });
                        }
                        Some(id) => {
                            let active = self.ws.active_env.as_ref() == Some(&id);
                            let Some(pos) = self.ws.environments.iter().position(|e| e.id == id) else {
                                self.env_selected = None;
                                return;
                            };
                            ui.horizontal(|ui| {
                                let env = &mut self.ws.environments[pos];
                                changed |= ui.add(egui::TextEdit::singleline(&mut env.name).font(egui::TextStyle::Heading).desired_width(260.0)).changed();
                                if active {
                                    widgets::chip(ui, "Active", p.ok);
                                } else if ui.button("Set active").clicked() {
                                    actions.push(Action::SetEnv(Some(id.clone())));
                                }
                                ui.menu_button("⋯", |ui| {
                                    if ui.button("Duplicate").clicked() {
                                        let mut copy = self.ws.environments[pos].clone();
                                        copy.id = tessera_core::model::new_id();
                                        copy.name = format!("{} Copy", copy.name);
                                        self.env_selected = Some(copy.id.clone());
                                        self.ws.environments.push(copy);
                                        changed = true;
                                        ui.close_menu();
                                    }
                                    if ui.button("Export (Postman)…").clicked() {
                                        actions.push(Action::ExportEnvironment(id.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button(RichText::new("Delete").color(p.err)).clicked() {
                                        self.ws.environments.remove(pos);
                                        if active {
                                            self.ws.active_env = None;
                                        }
                                        self.env_selected = None;
                                        changed = true;
                                        ui.close_menu();
                                    }
                                });
                            });
                            if let Some(env) = self.ws.environments.get_mut(pos) {
                                ui.add_space(6.0);
                                egui::ScrollArea::vertical().id_salt(("env", &env.id)).show(ui, |ui| {
                                    changed |= kv_table(ui, ("env-kv", &env.id), &mut env.variables, KvOptions { secrets: true, key_hint: "Variable", ..Default::default() });
                                });
                            }
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(dim(ui, "Use variables as {{name}} in URLs, headers, bodies and auth. Precedence: local → data → environment → collection → globals.").size(11.0));
                });
            });
        });
        if changed {
            self.mark_dirty();
        }
        self.actions.extend(actions);
        if !open {
            self.show_envs = false;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let before = (self.ws.settings.clone(), self.ws.certificates.clone());
        window("Settings").open(&mut open).default_size([680.0, 520.0]).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let s = &mut self.ws.settings;
                ui.label(RichText::new("General").strong());
                form_row(ui, "Theme", |ui| {
                    ui.radio_value(&mut s.dark_mode, true, "Dark");
                    ui.radio_value(&mut s.dark_mode, false, "Light");
                });
                form_row(ui, "UI scale", |ui| ui.add(egui::Slider::new(&mut s.ui_scale, 0.75..=1.75).step_by(0.05)));
                form_row(ui, "History size", |ui| ui.add(egui::DragValue::new(&mut s.max_history).range(10..=10_000)));
                ui.separator();
                ui.label(RichText::new("Requests").strong());
                form_row(ui, "Timeout (ms)", |ui| ui.add(egui::DragValue::new(&mut s.timeout_ms).range(0..=600_000).speed(100)));
                form_row(ui, "Follow redirects", |ui| ui.checkbox(&mut s.follow_redirects, ""));
                form_row(ui, "Verify TLS", |ui| ui.checkbox(&mut s.verify_tls, "Reject invalid / self-signed certificates"));
                form_row(ui, "User-Agent", |ui| ui.checkbox(&mut s.send_user_agent, "Send Tessera User-Agent when none is set"));
                form_row(ui, "Proxy", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.proxy).hint_text("http://user:pass@proxy:8080").desired_width(320.0));
                });
                form_row(ui, "CA certificate", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.ca_cert_path).hint_text("PEM file with extra trusted roots").desired_width(320.0));
                    if ui.button("…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            s.ca_cert_path = p.display().to_string();
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Client certificates").strong());
                    if ui.small_button("➕ Add").clicked() {
                        self.ws.certificates.push(ClientCert { enabled: true, ..Default::default() });
                    }
                });
                ui.label(dim(ui, "Presented for matching hosts (exact, host:port or *.domain). PEM needs PKCS#8 key; PFX/P12 with passphrase.").size(11.0));
                let mut remove = None;
                for (i, c) in self.ws.certificates.iter_mut().enumerate() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut c.enabled, "");
                            ui.add(egui::TextEdit::singleline(&mut c.host).hint_text("api.example.com").desired_width(200.0));
                            ui.radio_value(&mut c.kind, CertKind::Pem, "PEM");
                            ui.radio_value(&mut c.kind, CertKind::Pfx, "PFX");
                            if ui.button("🗑").clicked() {
                                remove = Some(i);
                            }
                        });
                        let path_row = |ui: &mut Ui, label: &str, value: &mut String| {
                            form_row(ui, label, |ui| {
                                ui.add(egui::TextEdit::singleline(value).desired_width(320.0));
                                if ui.button("…").clicked() {
                                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                                        *value = p.display().to_string();
                                    }
                                }
                            });
                        };
                        match c.kind {
                            CertKind::Pem => {
                                path_row(ui, "Certificate", &mut c.cert_path);
                                path_row(ui, "Private key", &mut c.key_path);
                            }
                            CertKind::Pfx => {
                                path_row(ui, "PFX file", &mut c.pfx_path);
                                form_row(ui, "Passphrase", |ui| ui.add(egui::TextEdit::singleline(&mut c.passphrase).password(true).desired_width(200.0)));
                            }
                        }
                    });
                }
                if let Some(i) = remove {
                    self.ws.certificates.remove(i);
                }
                ui.separator();
                ui.label(RichText::new("Data").strong());
                ui.label(dim(ui, format!("Stored locally in {}", tessera_core::storage::Storage::default_root().display())).size(11.5));
            });
        });
        if (self.ws.settings.clone(), self.ws.certificates.clone()) != before {
            self.bridge.engine.reset();
            self.mark_dirty();
        }
        if !open {
            self.show_settings = false;
        }
    }

    fn import_window(&mut self, ctx: &egui::Context) {
        if !self.show_import {
            return;
        }
        let mut open = true;
        let mut actions = Vec::new();
        window("Import").open(&mut open).default_size([620.0, 420.0]).show(ctx, |ui| {
            let p = pal(ui);
            egui::Frame::new().fill(p.surface).stroke(egui::Stroke::new(1.0_f32, p.border)).corner_radius(8).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("⤓").size(26.0).color(p.accent));
                    ui.label("Drop Postman collection / environment files anywhere in the window");
                    ui.add_space(4.0);
                    if ui.button("Choose files…").clicked() {
                        actions.push(Action::ImportFiles);
                    }
                });
            });
            ui.add_space(8.0);
            ui.label("…or paste a cURL command or Postman JSON:");
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                code_editor(ui, egui::Id::new("import-text"), &mut self.import_text, Lang::Plain, 10, None);
            });
            ui.horizontal(|ui| {
                let btn = egui::Button::new(RichText::new("Import").color(egui::Color32::WHITE)).fill(p.accent);
                if ui.add_enabled(!self.import_text.trim().is_empty(), btn).clicked() {
                    actions.push(Action::ImportText(self.import_text.clone()));
                }
                ui.label(dim(ui, "Supports Postman v2.0/v2.1 collections, environments, data dumps and cURL").size(11.5));
            });
        });
        self.actions.extend(actions);
        if !open {
            self.show_import = false;
        }
    }

    fn snippet_window(&mut self, ctx: &egui::Context) {
        let Some((tab_id, mut lang)) = self.snippet else { return };
        let Some(idx) = self.tab_index(tab_id) else {
            self.snippet = None;
            return;
        };
        let tab = &self.tabs[idx];
        let scope = self.scope_for(tab.collection_id.as_deref());
        let auth = match tab.collection_id.as_deref().and_then(|c| self.ws.collection(c)) {
            Some(c) => c.effective_auth(&tab.request),
            None if tab.request.auth.kind == tessera_core::model::AuthKind::Inherit => tessera_core::model::Auth::none(),
            None => tab.request.auth.clone(),
        };
        let prepared = tessera_core::http::prepare(&tab.request, &auth, &scope, &self.ws.settings);
        let mut code = snippet(&prepared, lang);
        let mut open = true;
        window("Code snippet").open(&mut open).default_size([640.0, 420.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("snippet-lang").selected_text(lang.label()).show_ui(ui, |ui| {
                    for l in SnippetLang::ALL {
                        ui.selectable_value(&mut lang, l, l.label());
                    }
                });
                if ui.button("📋 Copy").clicked() {
                    ui.ctx().copy_text(code.clone());
                    self.toasts.push(crate::state::Toast { text: "Copied".into(), error: false, until: std::time::Instant::now() + std::time::Duration::from_secs(2) });
                }
                ui.label(dim(ui, "Variables are resolved with the active environment").size(11.5));
            });
            egui::ScrollArea::both().show(ui, |ui| {
                let l = match lang {
                    SnippetLang::JavaScript => Lang::JavaScript,
                    _ => Lang::Plain,
                };
                code_editor(ui, egui::Id::new("snippet-code"), &mut code, l, 16, None);
            });
        });
        self.snippet = open.then_some((tab_id, lang));
    }

    fn item_settings_window(&mut self, ctx: &egui::Context) {
        let Some(sel) = self.item_settings.as_mut() else { return };
        let (cid, fid) = (sel.collection_id.clone(), sel.folder_id.clone());
        let mut section = sel.section;
        let scope = self.scope_for(Some(&cid));
        let busy = self.item_oauth_busy;
        let msg = self.item_oauth_msg.clone();
        let mut open = true;
        let mut changed = false;
        let mut actions = Vec::new();
        let Some(collection) = self.ws.collection_mut(&cid) else {
            self.item_settings = None;
            return;
        };
        let title = match &fid {
            Some(f) => format!("Folder settings · {}", collection.find_folder_mut(f).map(|f| f.name.clone()).unwrap_or_default()),
            None => format!("Collection settings · {}", collection.name),
        };
        window(&title).id(egui::Id::new("item-settings")).open(&mut open).default_size([720.0, 520.0]).show(ctx, |ui| {
            let mut items = vec![(ItemSection::Overview, "Overview".to_string()), (ItemSection::Auth, "Authorization".to_string()), (ItemSection::Scripts, "Scripts".to_string())];
            if fid.is_none() {
                items.push((ItemSection::Variables, format!("Variables ({})", collection.variables.iter().filter(|v| !v.key.is_empty()).count())));
            }
            widgets::tab_strip(ui, &mut section, &items);
            ui.add_space(8.0);
            egui::ScrollArea::vertical().show(ui, |ui| match &fid {
                None => {
                    let c = &mut *collection;
                    match section {
                        ItemSection::Overview => {
                            form_row(ui, "Name", |ui| changed |= ui.text_edit_singleline(&mut c.name).changed());
                            ui.label(dim(ui, "Description"));
                            changed |= ui.add(egui::TextEdit::multiline(&mut c.description).desired_rows(6).desired_width(f32::INFINITY)).changed();
                            ui.add_space(6.0);
                            ui.label(dim(ui, format!("{} requests", c.all_requests().len())));
                            ui.horizontal(|ui| {
                                if ui.button("▶ Run collection").clicked() {
                                    actions.push(Action::OpenRunner { collection_id: c.id.clone(), folder_id: None });
                                }
                                if ui.button("⤒ Export").clicked() {
                                    actions.push(Action::ExportCollection(c.id.clone()));
                                }
                            });
                        }
                        ItemSection::Auth => {
                            let before = c.auth.clone();
                            if let Some(a) = auth_editor(ui, &mut c.auth, &scope, false, AuthOwner::Collection(c.id.clone()), busy, msg.as_ref()) {
                                actions.push(a);
                            }
                            changed |= before != c.auth;
                        }
                        ItemSection::Scripts => {
                            let before = c.scripts.clone();
                            ui.label(dim(ui, "Runs before / after every request in this collection.").size(11.5));
                            scripts_editor(ui, &mut c.scripts.pre_request, &mut c.scripts.test, ("coll", &c.id));
                            changed |= before != c.scripts;
                        }
                        ItemSection::Variables => {
                            changed |= kv_table(ui, ("coll-vars", &c.id), &mut c.variables, KvOptions { secrets: true, key_hint: "Variable", ..Default::default() });
                        }
                    }
                }
                Some(fid) => {
                    let Some(f) = collection.find_folder_mut(fid) else { return };
                    match section {
                        ItemSection::Overview => {
                            form_row(ui, "Name", |ui| changed |= ui.text_edit_singleline(&mut f.name).changed());
                            ui.label(dim(ui, "Description"));
                            changed |= ui.add(egui::TextEdit::multiline(&mut f.description).desired_rows(6).desired_width(f32::INFINITY)).changed();
                            if ui.button("▶ Run folder").clicked() {
                                actions.push(Action::OpenRunner { collection_id: cid.clone(), folder_id: Some(f.id.clone()) });
                            }
                        }
                        ItemSection::Auth => {
                            let before = f.auth.clone();
                            if let Some(a) = auth_editor(ui, &mut f.auth, &scope, true, AuthOwner::Folder { collection: cid.clone(), folder: f.id.clone() }, busy, msg.as_ref()) {
                                actions.push(a);
                            }
                            changed |= before != f.auth;
                        }
                        ItemSection::Scripts => {
                            let before = f.scripts.clone();
                            ui.label(dim(ui, "Runs before / after every request in this folder.").size(11.5));
                            scripts_editor(ui, &mut f.scripts.pre_request, &mut f.scripts.test, ("folder", &f.id));
                            changed |= before != f.scripts;
                        }
                        ItemSection::Variables => {}
                    }
                }
            });
        });
        if changed {
            self.mark_dirty();
        }
        self.actions.extend(actions);
        match self.item_settings.as_mut() {
            Some(s) if open => s.section = section,
            _ => self.item_settings = None,
        }
    }

    fn save_as_window(&mut self, ctx: &egui::Context) {
        let Some(sa) = self.save_as.as_mut() else { return };
        let mut open = true;
        let mut commit = false;
        window("Save request").open(&mut open).resizable(false).default_width(420.0).show(ctx, |ui| {
            let p = pal(ui);
            form_row(ui, "Name", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut sa.name).desired_width(260.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
            });
            form_row(ui, "Collection", |ui| {
                let label = sa
                    .collection_id
                    .as_ref()
                    .and_then(|id| self.ws.collections.iter().find(|c| &c.id == id))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "➕ New collection".into());
                egui::ComboBox::from_id_salt("saveas-col").width(260.0).selected_text(label).show_ui(ui, |ui| {
                    for c in &self.ws.collections {
                        if ui.selectable_value(&mut sa.collection_id, Some(c.id.clone()), &c.name).changed() {
                            sa.folder_id = None;
                        }
                    }
                    ui.selectable_value(&mut sa.collection_id, None, "➕ New collection");
                });
            });
            match sa.collection_id.as_ref().and_then(|id| self.ws.collections.iter().find(|c| &c.id == id)) {
                None => {
                    form_row(ui, "New collection", |ui| ui.add(egui::TextEdit::singleline(&mut sa.new_collection).hint_text("My Collection").desired_width(260.0)));
                }
                Some(c) => {
                    let mut folders = Vec::new();
                    fn walk(items: &[Item], prefix: &str, out: &mut Vec<(String, String)>) {
                        for i in items {
                            if let Item::Folder(f) = i {
                                let path = format!("{prefix}{}", f.name);
                                out.push((f.id.clone(), path.clone()));
                                walk(&f.items, &format!("{path} / "), out);
                            }
                        }
                    }
                    walk(&c.items, "", &mut folders);
                    if !folders.is_empty() {
                        form_row(ui, "Folder", |ui| {
                            let label = sa.folder_id.as_ref().and_then(|id| folders.iter().find(|(fid, _)| fid == id)).map(|(_, n)| n.clone()).unwrap_or_else(|| "(collection root)".into());
                            egui::ComboBox::from_id_salt("saveas-folder").width(260.0).selected_text(label).show_ui(ui, |ui| {
                                ui.selectable_value(&mut sa.folder_id, None, "(collection root)");
                                for (id, name) in &folders {
                                    ui.selectable_value(&mut sa.folder_id, Some(id.clone()), name);
                                }
                            });
                        });
                    }
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let btn = egui::Button::new(RichText::new("Save").color(egui::Color32::WHITE)).fill(p.accent);
                if ui.add(btn).clicked() {
                    commit = true;
                }
            });
        });
        if commit {
            self.commit_save_as();
        } else if !open {
            self.save_as = None;
        }
    }

    fn confirm_delete_window(&mut self, ctx: &egui::Context) {
        let Some((id, name)) = self.confirm_delete.clone() else { return };
        let mut open = true;
        let mut decision = None;
        window("Delete").open(&mut open).resizable(false).show(ctx, |ui| {
            let p = pal(ui);
            ui.label(format!("Delete \"{name}\"? This can't be undone."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Delete").color(egui::Color32::WHITE)).fill(p.err)).clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });
        if decision == Some(true) {
            self.actions.push(Action::Delete { id });
        }
        if decision.is_some() || !open {
            self.confirm_delete = None;
        }
    }

    fn runner_window(&mut self, ctx: &egui::Context) {
        if !self.runner.open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut stop = false;
        let dark = self.ws.settings.dark_mode;
        window("Collection runner").id(egui::Id::new("runner")).open(&mut open).default_size([920.0, 560.0]).show(ctx, |ui| {
            let p = pal(ui);
            let r = &mut self.runner;
            ui.horizontal_top(|ui| {
                // ---- config ----
                ui.vertical(|ui| {
                    ui.set_width(270.0);
                    ui.label(RichText::new("Run configuration").strong());
                    ui.add_space(4.0);
                    let col_name = r.collection_id.as_ref().and_then(|id| self.ws.collections.iter().find(|c| &c.id == id)).map(|c| c.name.clone()).unwrap_or_else(|| "Select…".into());
                    ui.add_enabled_ui(!r.running, |ui| {
                        egui::ComboBox::from_id_salt("run-col").width(250.0).selected_text(col_name).show_ui(ui, |ui| {
                            for c in &self.ws.collections {
                                if ui.selectable_value(&mut r.collection_id, Some(c.id.clone()), &c.name).changed() {
                                    r.folder_id = None;
                                }
                            }
                        });
                        if let Some(c) = r.collection_id.as_ref().and_then(|id| self.ws.collections.iter().find(|c| &c.id == id)) {
                            let mut folders = Vec::new();
                            fn walk(items: &[Item], prefix: &str, out: &mut Vec<(String, String)>) {
                                for i in items {
                                    if let Item::Folder(f) = i {
                                        let path = format!("{prefix}{}", f.name);
                                        out.push((f.id.clone(), path.clone()));
                                        walk(&f.items, &format!("{path} / "), out);
                                    }
                                }
                            }
                            walk(&c.items, "", &mut folders);
                            if !folders.is_empty() {
                                let label = r.folder_id.as_ref().and_then(|id| folders.iter().find(|(f, _)| f == id)).map(|(_, n)| n.clone()).unwrap_or_else(|| "Whole collection".into());
                                egui::ComboBox::from_id_salt("run-folder").width(250.0).selected_text(label).show_ui(ui, |ui| {
                                    ui.selectable_value(&mut r.folder_id, None, "Whole collection");
                                    for (id, n) in &folders {
                                        ui.selectable_value(&mut r.folder_id, Some(id.clone()), n);
                                    }
                                });
                            }
                            let n = tessera_core::runner::runnable_requests(c, r.folder_id.as_deref()).len();
                            ui.label(dim(ui, format!("{n} requests")).size(11.5));
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Iterations");
                            ui.add(egui::DragValue::new(&mut r.iterations).range(0..=10_000));
                        });
                        if r.iterations == 0 {
                            ui.label(dim(ui, "0 = one per data row").size(11.0));
                        }
                        ui.horizontal(|ui| {
                            ui.label("Delay (ms)");
                            ui.add(egui::DragValue::new(&mut r.delay_ms).range(0..=60_000).speed(10));
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Data file…").on_hover_text("CSV (header row) or JSON array").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("Data", &["csv", "json"]).pick_file() {
                                    r.data_path = path.display().to_string();
                                    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| tessera_core::runner::parse_data_file(&t)) {
                                        Ok(d) => {
                                            r.data = d;
                                            r.data_error = None;
                                        }
                                        Err(e) => {
                                            r.data.clear();
                                            r.data_error = Some(e);
                                        }
                                    }
                                }
                            }
                            if !r.data_path.is_empty() && ui.small_button("✕").clicked() {
                                r.data_path.clear();
                                r.data.clear();
                                r.data_error = None;
                            }
                        });
                        if !r.data_path.is_empty() {
                            let name = std::path::Path::new(&r.data_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            match &r.data_error {
                                Some(e) => ui.label(RichText::new(e).color(p.err).size(11.5)),
                                None => ui.label(dim(ui, format!("{name} · {} rows", r.data.len())).size(11.5)),
                            };
                        }
                        ui.checkbox(&mut r.stop_on_error, "Stop on first failure");
                        ui.checkbox(&mut r.persist_vars, "Keep variable changes");
                    });
                    ui.add_space(8.0);
                    if r.running {
                        if ui.add(egui::Button::new(RichText::new("■ Stop").color(egui::Color32::WHITE)).fill(p.err).min_size(egui::vec2(250.0, 30.0))).clicked() {
                            stop = true;
                        }
                    } else {
                        let ok = r.collection_id.is_some();
                        if ui.add_enabled(ok, egui::Button::new(RichText::new("▶ Run").color(egui::Color32::WHITE)).fill(p.accent).min_size(egui::vec2(250.0, 30.0))).clicked() {
                            start = true;
                        }
                    }
                });
                ui.separator();
                // ---- results ----
                ui.vertical(|ui| {
                    let done = r.results.len();
                    let failed = r.results.iter().filter(|x| x.error.is_some() || x.tests.iter().any(|t| !t.passed)).count();
                    let tests_total: usize = r.results.iter().map(|x| x.tests.len()).sum();
                    let tests_passed: usize = r.results.iter().map(|x| x.tests.iter().filter(|t| t.passed).count()).sum();
                    let avg = if done > 0 { r.results.iter().map(|x| x.time_ms).sum::<f64>() / done as f64 } else { 0.0 };
                    ui.horizontal(|ui| {
                        widgets::chip(ui, format!("{done}/{} requests", r.total.max(done)), p.info);
                        widgets::chip(ui, format!("{} passed", done - failed), p.ok);
                        if failed > 0 {
                            widgets::chip(ui, format!("{failed} failed"), p.err);
                        }
                        widgets::chip(ui, format!("tests {tests_passed}/{tests_total}"), p.accent);
                        ui.label(dim(ui, format!("avg {}", widgets::human_time(avg))));
                        let elapsed = if r.running { r.started.map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0) } else { r.elapsed_ms };
                        if elapsed > 0.0 {
                            ui.label(dim(ui, format!("· total {}", widgets::human_time(elapsed))));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut r.only_failed, "Failed only");
                        });
                    });
                    if r.total > 0 {
                        ui.add(egui::ProgressBar::new(done as f32 / r.total as f32).desired_height(4.0));
                    }
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().id_salt("run-results").auto_shrink([false; 2]).stick_to_bottom(r.running).show(ui, |ui| {
                        for (i, res) in r.results.iter().enumerate() {
                            let bad = res.error.is_some() || res.tests.iter().any(|t| !t.passed);
                            if r.only_failed && !bad {
                                continue;
                            }
                            let selected = r.selected == Some(i);
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.label(RichText::new(if bad { "✖" } else { "✔" }).color(if bad { p.err } else { p.ok }));
                                    if r.iterations != 1 || r.data.len() > 1 {
                                        ui.label(dim(ui, format!("#{}", res.iteration + 1)).size(11.0));
                                    }
                                    ui.label(widgets::method_text(&res.method, dark, 11.0));
                                    ui.label(RichText::new(&res.name).strong());
                                    match (res.status, &res.error) {
                                        (Some(s), _) => {
                                            ui.label(RichText::new(s.to_string()).color(theme::status_color(s, dark)));
                                        }
                                        (None, Some(e)) => {
                                            ui.label(RichText::new(e).color(p.err).size(11.5));
                                        }
                                        _ => {}
                                    }
                                    ui.label(dim(ui, format!("{} · {}", widgets::human_time(res.time_ms), widgets::human_size(res.size))).size(11.5));
                                    if !res.tests.is_empty() {
                                        let passed = res.tests.iter().filter(|t| t.passed).count();
                                        ui.label(dim(ui, format!("tests {passed}/{}", res.tests.len())).size(11.5));
                                    }
                                })
                                .response
                                .interact(egui::Sense::click());
                            if resp.clicked() {
                                r.selected = if selected { None } else { Some(i) };
                            }
                            if selected || (bad && res.tests.iter().any(|t| !t.passed)) {
                                ui.indent(("res", i), |ui| {
                                    ui.label(dim(ui, &res.url).monospace().size(11.5));
                                    for t in &res.tests {
                                        if selected || !t.passed {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(if t.passed { "✔" } else { "✖" }).color(if t.passed { p.ok } else { p.err }).size(11.5));
                                                ui.label(RichText::new(&t.name).size(12.0));
                                                if !t.passed {
                                                    ui.label(RichText::new(&t.error).color(p.err).size(11.5));
                                                }
                                            });
                                        }
                                    }
                                });
                            }
                            ui.separator();
                        }
                        if r.results.is_empty() && !r.running {
                            widgets::empty_state(ui, "▶", "Nothing run yet", "Pick a collection and press Run");
                        }
                    });
                });
            });
        });
        if start {
            self.start_run();
        }
        if stop {
            self.stop_run();
        }
        if !open {
            self.runner.open = false;
        }
    }
}
