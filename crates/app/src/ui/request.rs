//! Request editor: URL bar and the request sections.

use egui::{RichText, Ui};
use tessera_core::graphql::{base_type, Schema};
use tessera_core::model::{
    ApiKeyIn, Auth, AuthKind, BodyMode, OAuthGrant, RawLang, Request, RequestKind, METHODS,
};
use tessera_core::vars::{join_query, sync_params_from_url, VarScope};

use crate::app::Action;
use crate::bridge::AuthOwner;
use crate::state::{ReqSection, Tab};
use crate::theme;
use crate::ui::widgets::{self, code_editor, dim, form_row, kv_table, pal, var_input, KvOptions, Lang};

pub fn url_id(tab_id: u64) -> egui::Id {
    egui::Id::new(("url", tab_id))
}

pub fn url_bar(ui: &mut Ui, tab: &mut Tab, scope: &VarScope, actions: &mut Vec<Action>) {
    let p = pal(ui);
    let dark = ui.visuals().dark_mode;
    let req = &mut tab.request;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        match req.kind {
            RequestKind::Http => {
                egui::ComboBox::from_id_salt(("method", tab.id))
                    .width(86.0)
                    .selected_text(widgets::method_text(&req.method, dark, 13.0))
                    .show_ui(ui, |ui| {
                        for m in METHODS {
                            ui.selectable_value(&mut req.method, m.to_string(), widgets::method_text(m, dark, 13.0));
                        }
                        ui.separator();
                        ui.label(dim(ui, "Custom").size(11.0));
                        let mut custom = if METHODS.contains(&req.method.as_str()) { String::new() } else { req.method.clone() };
                        if ui.add(egui::TextEdit::singleline(&mut custom).hint_text("e.g. PROPFIND").desired_width(90.0)).changed() {
                            req.method = custom.to_uppercase();
                        }
                    });
            }
            RequestKind::GraphQL => {
                widgets::chip(ui, "GraphQL", theme::method_color("GQL", dark));
            }
            RequestKind::WebSocket => {
                widgets::chip(ui, "WebSocket", theme::method_color("WS", dark));
            }
        }

        let busy = tab.pending.is_some() || tab.ws.connecting;
        let button_w = if req.kind == RequestKind::GraphQL { 300.0 } else { 210.0 };
        let url_w = (ui.available_width() - button_w).max(120.0);
        let hint = match req.kind {
            RequestKind::WebSocket => "wss://echo.websocket.org",
            _ => "https://api.example.com/users   — or paste a cURL command",
        };
        let resp = var_input(ui, &mut req.url, hint, url_w, scope, Some(url_id(tab.id)));
        if resp.changed() {
            if req.url.trim_start().starts_with("curl ") {
                match tessera_core::curl::parse_curl(&req.url) {
                    Ok(mut parsed) => {
                        parsed.id = req.id.clone();
                        if !req.name.starts_with("New ") && !req.name.is_empty() {
                            parsed.name = req.name.clone();
                        }
                        *req = parsed;
                        actions.push(Action::Toast("Imported cURL command".into(), false));
                    }
                    Err(e) => actions.push(Action::Toast(e, true)),
                }
            } else {
                sync_params_from_url(&req.url.clone(), &mut req.params);
            }
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            actions.push(Action::Send(tab.id));
        }

        match req.kind {
            RequestKind::WebSocket => {
                let (label, color) = if tab.ws.connected || tab.ws.connecting { ("Disconnect", p.err) } else { ("Connect", p.accent) };
                let btn = egui::Button::new(RichText::new(label).color(egui::Color32::WHITE).strong()).fill(color).min_size(egui::vec2(96.0, 28.0));
                if ui.add(btn).clicked() {
                    actions.push(Action::Send(tab.id));
                }
            }
            _ => {
                if busy {
                    let btn = egui::Button::new(RichText::new("Cancel").color(egui::Color32::WHITE).strong()).fill(p.err).min_size(egui::vec2(80.0, 28.0));
                    if ui.add(btn).clicked() {
                        actions.push(Action::Cancel(tab.id));
                    }
                } else {
                    let btn = egui::Button::new(RichText::new("Send").color(egui::Color32::WHITE).strong()).fill(p.accent).min_size(egui::vec2(80.0, 28.0));
                    if ui.add(btn).on_hover_text("Ctrl+Enter").clicked() {
                        actions.push(Action::Send(tab.id));
                    }
                }
            }
        }
        if req.kind == RequestKind::GraphQL {
            let label = if tab.schema_loading { "Loading…" } else { "Schema" };
            if ui.add_enabled(!tab.schema_loading, egui::Button::new(label).min_size(egui::vec2(0.0, 28.0))).on_hover_text("Fetch schema via introspection").clicked() {
                actions.push(Action::FetchSchema(tab.id));
            }
        }
        if ui.add(egui::Button::new("Save").min_size(egui::vec2(0.0, 28.0))).on_hover_text("Save (Ctrl+S)").clicked() {
            actions.push(Action::Save(tab.id));
        }
        if req.kind != RequestKind::WebSocket
            && ui.add(egui::Button::new("</>").min_size(egui::vec2(30.0, 28.0))).on_hover_text("Code snippet").clicked()
        {
            actions.push(Action::Snippet(tab.id));
        }
    });
}

pub fn sections(ui: &mut Ui, tab: &mut Tab, scope: &VarScope, actions: &mut Vec<Action>) {
    let req = &tab.request;
    let count = |n: usize| if n == 0 { String::new() } else { format!(" ({n})") };
    let n_params = req.params.iter().filter(|p| p.enabled && !p.key.is_empty()).count();
    let n_headers = req.headers.iter().filter(|p| p.enabled && !p.key.is_empty()).count();
    let has_scripts = !req.scripts.pre_request.trim().is_empty() || !req.scripts.test.trim().is_empty();
    let auth_label = if req.auth.kind == AuthKind::Inherit || req.auth.kind == AuthKind::None { "Auth".to_string() } else { "Auth •".to_string() };
    let body_label = if req.body.mode == BodyMode::None { "Body".to_string() } else { "Body •".to_string() };
    let scripts_label = if has_scripts { "Scripts •".to_string() } else { "Scripts".to_string() };
    let items: Vec<(ReqSection, String)> = match req.kind {
        RequestKind::Http => vec![
            (ReqSection::Params, format!("Params{}", count(n_params))),
            (ReqSection::Auth, auth_label),
            (ReqSection::Headers, format!("Headers{}", count(n_headers))),
            (ReqSection::Body, body_label),
            (ReqSection::Scripts, scripts_label),
            (ReqSection::Settings, "Settings".into()),
        ],
        RequestKind::GraphQL => vec![
            (ReqSection::Query, "Query".into()),
            (ReqSection::Variables, "Variables".into()),
            (ReqSection::Auth, auth_label),
            (ReqSection::Headers, format!("Headers{}", count(n_headers))),
            (ReqSection::Scripts, scripts_label),
            (ReqSection::Settings, "Settings".into()),
        ],
        RequestKind::WebSocket => vec![
            (ReqSection::Messages, "Message".into()),
            (ReqSection::Params, format!("Params{}", count(n_params))),
            (ReqSection::Headers, format!("Headers{}", count(n_headers))),
            (ReqSection::Auth, auth_label),
            (ReqSection::Settings, "Settings".into()),
        ],
    };
    if !items.iter().any(|(s, _)| *s == tab.req_section) {
        tab.req_section = items[0].0;
    }
    widgets::tab_strip(ui, &mut tab.req_section, &items);
    ui.add_space(6.0);

    if tab.request.kind == RequestKind::GraphQL {
        if let Some(schema) = &tab.schema {
            let mut insert: Option<String> = None;
            egui::SidePanel::right(egui::Id::new(("schema", tab.id)))
                .resizable(true)
                .default_width(260.0)
                .width_range(180.0..=500.0)
                .show_inside(ui, |ui| match schema {
                    Ok(s) => schema_panel(ui, s, &mut tab.schema_filter, &mut insert),
                    Err(e) => {
                        ui.label(RichText::new(e).color(pal(ui).err));
                    }
                });
            if let Some(text) = insert {
                let q = &mut tab.request.body.graphql_query;
                if !q.is_empty() && !q.ends_with(char::is_whitespace) {
                    q.push(' ');
                }
                q.push_str(&text);
            }
        }
    }

    egui::ScrollArea::vertical().id_salt(("req-scroll", tab.id)).auto_shrink([false; 2]).show(ui, |ui| {
        let tab_id = tab.id;
        let req = &mut tab.request;
        match tab.req_section {
            ReqSection::Params => {
                let opts = KvOptions { key_hint: "Query parameter", scope: Some(scope), ..Default::default() };
                if kv_table(ui, ("params", tab_id), &mut req.params, opts) {
                    req.url = join_query(&req.url, &req.params);
                }
            }
            ReqSection::Headers => {
                let opts = KvOptions { key_hint: "Header", scope: Some(scope), ..Default::default() };
                kv_table(ui, ("headers", tab_id), &mut req.headers, opts);
                ui.add_space(4.0);
                ui.label(dim(ui, "Auto-added if missing: User-Agent, Accept, Content-Type (from body)").size(11.0));
            }
            ReqSection::Auth => {
                if let Some(a) = auth_editor(ui, &mut req.auth, scope, true, AuthOwner::Tab(tab_id), tab.oauth_busy, tab.oauth_msg.as_ref()) {
                    actions.push(a);
                }
            }
            ReqSection::Body => body_editor(ui, req, scope, tab_id),
            ReqSection::Scripts => scripts_editor(ui, &mut req.scripts.pre_request, &mut req.scripts.test, tab_id),
            ReqSection::Settings => request_settings(ui, req),
            ReqSection::Query => {
                code_editor(ui, egui::Id::new(("gql-q", tab_id)), &mut req.body.graphql_query, Lang::GraphQL, 14, Some(scope));
            }
            ReqSection::Variables => {
                ui.label(dim(ui, "Variables as JSON").size(11.5));
                code_editor(ui, egui::Id::new(("gql-v", tab_id)), &mut req.body.graphql_variables, Lang::Json, 10, Some(scope));
                if ui.small_button("Beautify").clicked() {
                    beautify(&mut req.body.graphql_variables);
                }
            }
            ReqSection::Messages => ws_composer(ui, tab, scope, actions),
        }
    });
}

fn beautify(text: &mut String) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            *text = pretty;
        }
    }
}

fn body_editor(ui: &mut Ui, req: &mut Request, scope: &VarScope, tab_id: u64) {
    let body = &mut req.body;
    ui.horizontal(|ui| {
        for (mode, label) in [
            (BodyMode::None, "none"),
            (BodyMode::Raw, "raw"),
            (BodyMode::FormUrlEncoded, "x-www-form-urlencoded"),
            (BodyMode::Multipart, "form-data"),
            (BodyMode::Binary, "binary"),
        ] {
            ui.radio_value(&mut body.mode, mode, label);
        }
        if body.mode == BodyMode::Raw {
            ui.separator();
            egui::ComboBox::from_id_salt(("rawlang", tab_id))
                .width(96.0)
                .selected_text(body.raw_lang.label())
                .show_ui(ui, |ui| {
                    for l in RawLang::ALL {
                        ui.selectable_value(&mut body.raw_lang, l, l.label());
                    }
                });
            if body.raw_lang == RawLang::Json && ui.small_button("Beautify").clicked() {
                beautify(&mut body.raw);
            }
        }
    });
    ui.add_space(4.0);
    match body.mode {
        BodyMode::None => {
            ui.label(dim(ui, "This request has no body"));
        }
        BodyMode::Raw => {
            let lang = match body.raw_lang {
                RawLang::Json => Lang::Json,
                RawLang::JavaScript => Lang::JavaScript,
                RawLang::Xml | RawLang::Html => Lang::Markup,
                RawLang::Text => Lang::Plain,
            };
            code_editor(ui, egui::Id::new(("raw", tab_id)), &mut body.raw, lang, 12, Some(scope));
            if body.raw_lang == RawLang::Json && !body.raw.trim().is_empty() && !body.raw.contains("{{") {
                if let Err(e) = serde_json::from_str::<serde_json::Value>(&body.raw) {
                    ui.label(RichText::new(format!("⚠ Invalid JSON: {e}")).color(pal(ui).warn).size(11.5));
                }
            }
        }
        BodyMode::FormUrlEncoded => {
            kv_table(ui, ("form", tab_id), &mut body.form, KvOptions { scope: Some(scope), ..Default::default() });
        }
        BodyMode::Multipart => {
            kv_table(ui, ("multipart", tab_id), &mut body.multipart, KvOptions { allow_files: true, scope: Some(scope), ..Default::default() });
        }
        BodyMode::Binary => {
            ui.horizontal(|ui| {
                if ui.button("Choose file…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        body.binary_path = p.display().to_string();
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut body.binary_path).hint_text("Path to file").desired_width(f32::INFINITY));
            });
        }
    }
}

const TEST_SNIPPETS: &[(&str, &str)] = &[
    ("Status code is 200", "pm.test(\"Status code is 200\", function () {\n    pm.response.to.have.status(200);\n});\n"),
    ("Response time < 500ms", "pm.test(\"Response time is less than 500ms\", function () {\n    pm.expect(pm.response.responseTime).to.be.below(500);\n});\n"),
    ("Body is JSON", "pm.test(\"Body is JSON\", function () {\n    pm.response.to.be.json;\n});\n"),
    ("Check JSON value", "pm.test(\"Check value\", function () {\n    const json = pm.response.json();\n    pm.expect(json.id).to.eql(1);\n});\n"),
    ("Body contains string", "pm.test(\"Body contains string\", function () {\n    pm.expect(pm.response.text()).to.include(\"string\");\n});\n"),
    ("Header present", "pm.test(\"Content-Type is present\", function () {\n    pm.response.to.have.header(\"Content-Type\");\n});\n"),
    ("Save token to env", "const json = pm.response.json();\npm.environment.set(\"token\", json.token);\n"),
];

const PRE_SNIPPETS: &[(&str, &str)] = &[
    ("Set env variable", "pm.environment.set(\"key\", \"value\");\n"),
    ("Set timestamp var", "pm.variables.set(\"ts\", Date.now());\n"),
    ("Add header", "pm.request.headers.add({ key: \"X-Request-Id\", value: pm.variables.replaceIn(\"{{$guid}}\") });\n"),
    ("Log a variable", "console.log(pm.variables.get(\"key\"));\n"),
];

pub fn scripts_editor(ui: &mut Ui, pre: &mut String, test: &mut String, salt: impl std::hash::Hash + Copy) {
    let id = ui.make_persistent_id(("script-tab", salt));
    // Open on the script that has content (tests are the common case).
    let default = if pre.trim().is_empty() && !test.trim().is_empty() { 1 } else { 0 };
    let mut which: u8 = ui.data(|d| d.get_temp(id)).unwrap_or(default);
    let pre_label = if pre.trim().is_empty() { "Pre-request" } else { "Pre-request •" };
    let test_label = if test.trim().is_empty() { "Tests" } else { "Tests •" };
    ui.horizontal(|ui| {
        widgets::segmented(ui, &mut which, &[(0u8, pre_label), (1u8, test_label)]);
        ui.label(dim(ui, "JavaScript · pm.* API compatible with Postman").size(11.5));
    });
    ui.data_mut(|d| d.insert_temp(id, which));
    ui.add_space(4.0);
    let code = if which == 0 { pre } else { test };
    let snippets = if which == 0 { PRE_SNIPPETS } else { TEST_SNIPPETS };
    ui.horizontal_top(|ui| {
        let w = (ui.available_width() - 190.0).max(200.0);
        ui.vertical(|ui| {
            ui.set_width(w);
            code_editor(ui, egui::Id::new(("script", which, ui.id())), code, Lang::JavaScript, 14, None);
        });
        ui.vertical(|ui| {
            ui.label(dim(ui, "Snippets").size(11.5));
            for (label, snippet) in snippets {
                if ui.add(egui::Button::new(RichText::new(*label).size(12.0)).frame(false)).clicked() {
                    if !code.is_empty() && !code.ends_with('\n') {
                        code.push('\n');
                    }
                    code.push_str(snippet);
                }
            }
        });
    });
}

fn request_settings(ui: &mut Ui, req: &mut Request) {
    let s = &mut req.settings;
    form_row(ui, "Timeout (ms)", |ui| {
        ui.add(egui::DragValue::new(&mut s.timeout_ms).range(0..=600_000).speed(100));
        ui.label(dim(ui, "0 = use global default"));
    });
    form_row(ui, "Follow redirects", |ui| ui.checkbox(&mut s.follow_redirects, ""));
    form_row(ui, "Verify TLS", |ui| ui.checkbox(&mut s.verify_tls, ""));
    ui.add_space(8.0);
    ui.label(dim(ui, "Description"));
    ui.add(egui::TextEdit::multiline(&mut req.description).desired_rows(4).desired_width(f32::INFINITY).hint_text("Notes about this request (Markdown welcome)"));
}

/// Auth editor shared by requests, folders and collections. Returns a token action when requested.
pub fn auth_editor(
    ui: &mut Ui,
    auth: &mut Auth,
    scope: &VarScope,
    allow_inherit: bool,
    owner: AuthOwner,
    busy: bool,
    msg: Option<&Result<String, String>>,
) -> Option<Action> {
    let p = pal(ui);
    let mut action = None;
    form_row(ui, "Type", |ui| {
        egui::ComboBox::from_id_salt(("authkind", format!("{owner:?}")))
            .width(200.0)
            .selected_text(auth.kind.label())
            .show_ui(ui, |ui| {
                for k in AuthKind::ALL {
                    if k == AuthKind::Inherit && !allow_inherit {
                        continue;
                    }
                    ui.selectable_value(&mut auth.kind, k, k.label());
                }
            });
    });
    ui.add_space(6.0);
    let field = |ui: &mut Ui, label: &str, value: &mut String, password: bool| {
        form_row(ui, label, |ui| {
            if password {
                ui.add(egui::TextEdit::singleline(value).password(true).desired_width(360.0));
            } else {
                var_input(ui, value, "", 360.0, scope, None);
            }
        });
    };
    match auth.kind {
        AuthKind::Inherit => {
            ui.label(dim(ui, "Uses the auth configured on the parent folder or collection."));
        }
        AuthKind::None => {
            ui.label(dim(ui, "No authorization header will be sent."));
        }
        AuthKind::Bearer => field(ui, "Token", &mut auth.bearer_token, false),
        AuthKind::Basic => {
            field(ui, "Username", &mut auth.basic_user, false);
            field(ui, "Password", &mut auth.basic_pass, false);
        }
        AuthKind::ApiKey => {
            field(ui, "Key", &mut auth.apikey_key, false);
            field(ui, "Value", &mut auth.apikey_value, false);
            form_row(ui, "Add to", |ui| {
                ui.radio_value(&mut auth.apikey_in, ApiKeyIn::Header, "Header");
                ui.radio_value(&mut auth.apikey_in, ApiKeyIn::Query, "Query params");
            });
        }
        AuthKind::OAuth2 => {
            let o = &mut auth.oauth2;
            form_row(ui, "Access token", |ui| {
                var_input(ui, &mut o.access_token, "Paste a token or fetch one below", 360.0, scope, None);
            });
            if o.expires_at > 0 {
                let left = o.expires_at - chrono::Utc::now().timestamp();
                let text = if left > 0 { format!("expires in {} min", left / 60) } else { "expired".into() };
                form_row(ui, "", |ui| ui.label(RichText::new(text).color(if left > 0 { p.dim } else { p.warn }).size(11.5)));
            }
            form_row(ui, "Header prefix", |ui| ui.add(egui::TextEdit::singleline(&mut o.header_prefix).desired_width(120.0)));
            ui.separator();
            ui.label(RichText::new("Configure new token").strong());
            form_row(ui, "Grant type", |ui| {
                egui::ComboBox::from_id_salt(("grant", format!("{owner:?}")))
                    .width(220.0)
                    .selected_text(o.grant.label())
                    .show_ui(ui, |ui| {
                        for g in OAuthGrant::ALL {
                            ui.selectable_value(&mut o.grant, g, g.label());
                        }
                    });
            });
            if o.grant == OAuthGrant::AuthorizationCode {
                field(ui, "Auth URL", &mut o.auth_url, false);
            }
            field(ui, "Token URL", &mut o.token_url, false);
            field(ui, "Client ID", &mut o.client_id, false);
            field(ui, "Client secret", &mut o.client_secret, false);
            field(ui, "Scope", &mut o.scope, false);
            if o.grant == OAuthGrant::Password {
                field(ui, "Username", &mut o.username, false);
                field(ui, "Password", &mut o.password, true);
            }
            if o.grant == OAuthGrant::AuthorizationCode {
                form_row(ui, "Callback port", |ui| {
                    ui.add(egui::DragValue::new(&mut o.redirect_port).range(1024..=65535));
                    ui.label(dim(ui, tessera_core::oauth::redirect_uri(o.redirect_port)).size(11.5))
                        .on_hover_text("Register this redirect URI with your OAuth provider");
                });
                form_row(ui, "PKCE", |ui| ui.checkbox(&mut o.use_pkce, "Use PKCE (S256)"));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(138.0);
                if busy {
                    ui.spinner();
                    ui.label("Waiting for token…");
                } else {
                    let btn = egui::Button::new(RichText::new("Get new access token").color(egui::Color32::WHITE)).fill(p.accent);
                    if ui.add(btn).clicked() {
                        action = Some(Action::FetchToken { owner: owner.clone(), refresh: false });
                    }
                    if !o.refresh_token.is_empty() && ui.button("Refresh").clicked() {
                        action = Some(Action::FetchToken { owner: owner.clone(), refresh: true });
                    }
                    if !o.access_token.is_empty() && ui.button("Clear").clicked() {
                        o.access_token.clear();
                        o.refresh_token.clear();
                        o.expires_at = 0;
                    }
                }
            });
            if let Some(m) = msg {
                let (text, color) = match m {
                    Ok(t) => (t.as_str(), p.ok),
                    Err(e) => (e.as_str(), p.err),
                };
                ui.label(RichText::new(text).color(color).size(12.0));
            }
        }
    }
    action
}

fn schema_panel(ui: &mut Ui, schema: &Schema, filter: &mut String, insert: &mut Option<String>) {
    let p = pal(ui);
    ui.add(egui::TextEdit::singleline(filter).hint_text("🔍 Filter schema").desired_width(f32::INFINITY));
    let f = filter.to_lowercase();
    egui::ScrollArea::vertical().id_salt("schema-scroll").auto_shrink([false; 2]).show(ui, |ui| {
        for (label, t) in schema.roots() {
            egui::CollapsingHeader::new(RichText::new(format!("{label} ({})", t.fields.len())).strong()).default_open(true).show(ui, |ui| {
                for field in t.fields.iter().filter(|fl| f.is_empty() || fl.name.to_lowercase().contains(&f)) {
                    let args = if field.args.is_empty() {
                        String::new()
                    } else {
                        format!("({})", field.args.iter().map(|(n, t)| format!("{n}: {t}")).collect::<Vec<_>>().join(", "))
                    };
                    let r = ui.add(egui::Label::new(RichText::new(format!("{}{args}", field.name)).monospace().color(p.json_key)).sense(egui::Sense::click()));
                    let r = r.on_hover_ui(|ui| {
                        ui.label(RichText::new(format!("→ {}", field.type_name)).monospace());
                        if !field.description.is_empty() {
                            ui.label(&field.description);
                        }
                        if let Some(ty) = schema.get(base_type(&field.type_name)) {
                            if !ty.fields.is_empty() {
                                ui.separator();
                                for sub in ty.fields.iter().take(25) {
                                    ui.label(RichText::new(format!("{}: {}", sub.name, sub.type_name)).monospace().size(11.5));
                                }
                            }
                        }
                        ui.label(dim(ui, "Click to insert").size(11.0));
                    });
                    if r.clicked() {
                        // Insert the field with its scalar sub-fields as a starting selection.
                        let mut text = field.name.clone();
                        if let Some(ty) = schema.get(base_type(&field.type_name)) {
                            let scalars: Vec<&str> = ty
                                .fields
                                .iter()
                                .filter(|s| s.args.is_empty() && schema.get(base_type(&s.type_name)).map(|t| t.kind == "SCALAR" || t.kind == "ENUM").unwrap_or(true))
                                .take(8)
                                .map(|s| s.name.as_str())
                                .collect();
                            if ty.kind == "OBJECT" && !scalars.is_empty() {
                                text = format!("{text} {{ {} }}", scalars.join(" "));
                            }
                        }
                        *insert = Some(text);
                    }
                }
            });
        }
        egui::CollapsingHeader::new(RichText::new(format!("Types ({})", schema.types.len())).strong()).show(ui, |ui| {
            for t in schema.types.iter().filter(|t| f.is_empty() || t.name.to_lowercase().contains(&f)) {
                egui::CollapsingHeader::new(RichText::new(&t.name).monospace()).id_salt(("t", &t.name)).show(ui, |ui| {
                    ui.label(dim(ui, &t.kind).size(11.0));
                    for fl in &t.fields {
                        ui.label(RichText::new(if fl.type_name.is_empty() { fl.name.clone() } else { format!("{}: {}", fl.name, fl.type_name) }).monospace().size(12.0));
                    }
                });
            }
        });
    });
}

fn ws_composer(ui: &mut Ui, tab: &mut Tab, scope: &VarScope, actions: &mut Vec<Action>) {
    let p = pal(ui);
    let connected = tab.ws.connected;
    code_editor(ui, egui::Id::new(("ws-compose", tab.id)), &mut tab.ws.compose, Lang::Json, 6, Some(scope));
    ui.horizontal(|ui| {
        let btn = egui::Button::new(RichText::new("Send message").color(egui::Color32::WHITE)).fill(if connected { p.accent } else { p.surface_hi });
        if ui.add_enabled(connected, btn).on_hover_text("Ctrl+Enter").clicked() {
            actions.push(Action::WsSend(tab.id));
        }
        if !connected {
            ui.label(dim(ui, "Connect first"));
        }
        if ui.small_button("Beautify").clicked() {
            beautify(&mut tab.ws.compose);
        }
    });
}

/// WebSocket message log (shown in the lower pane).
pub fn ws_log(ui: &mut Ui, tab: &mut Tab) {
    let p = pal(ui);
    ui.horizontal(|ui| {
        let (text, color) = if tab.ws.connected {
            ("Connected", p.ok)
        } else if tab.ws.connecting {
            ("Connecting…", p.warn)
        } else {
            ("Disconnected", p.dim)
        };
        widgets::chip(ui, text, color);
        ui.label(dim(ui, format!("{} messages", tab.ws.log.iter().filter(|l| !l.system).count())));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Clear").clicked() {
                tab.ws.log.clear();
            }
            ui.add(egui::TextEdit::singleline(&mut tab.ws.filter).hint_text("🔍 Filter").desired_width(160.0));
        });
    });
    ui.add_space(4.0);
    if tab.ws.log.is_empty() {
        widgets::empty_state(ui, "⇅", "No messages yet", "Connect, then send a message from the Message tab");
        return;
    }
    let f = tab.ws.filter.to_lowercase();
    egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
        for line in tab.ws.log.iter().filter(|l| f.is_empty() || l.text.to_lowercase().contains(&f)) {
            let (arrow, color) = if line.system {
                ("•", p.dim)
            } else if line.incoming {
                ("↓", p.ok)
            } else {
                ("↑", p.info)
            };
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(&line.at).monospace().size(11.0).color(p.dim));
                ui.label(RichText::new(arrow).strong().color(color));
                let text = if line.system { RichText::new(&line.text).italics().color(p.dim) } else { RichText::new(&line.text).monospace() };
                let r = ui.add(egui::Label::new(text).selectable(true).wrap());
                r.context_menu(|ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(line.text.clone());
                        ui.close_menu();
                    }
                });
            });
            ui.separator();
        }
    });
}
