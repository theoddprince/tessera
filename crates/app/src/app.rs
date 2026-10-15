//! Application state, action processing and top-level layout.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Key, KeyboardShortcut, Modifiers, RichText};
use tessera_core::curl::SnippetLang;
use tessera_core::model::*;
use tessera_core::postman::{self, Imported};
use tessera_core::script::{ScriptInfo, ScriptVars};
use tessera_core::send::{send, SendContext};
use tessera_core::storage::Storage;
use tessera_core::vars::VarScope;
use tessera_core::ws::{WsCommand, WsEvent};

use crate::bridge::{AppEvent, AuthOwner, Bridge};
use crate::state::*;
use crate::theme;
use crate::ui::sidebar::{self, SidebarCtx, SidebarMode};
use crate::ui::widgets::{self, dim, pal};
use crate::ui::{request, response};

#[derive(Clone, Debug)]
pub enum Action {
    Send(u64),
    Cancel(u64),
    Save(u64),
    Snippet(u64),
    FetchSchema(u64),
    WsSend(u64),
    FetchToken { owner: AuthOwner, refresh: bool },
    Toast(String, bool),
    NewTab(RequestKind),
    CloseTab(u64),
    CloseOtherTabs(u64),
    DuplicateTab(u64),
    OpenRequest { collection_id: String, request_id: String },
    OpenHistory(usize),
    DeleteHistory(usize),
    ClearHistory,
    NewCollection,
    AddRequest { collection_id: String, container: String, kind: RequestKind },
    AddFolder { collection_id: String, container: String },
    Rename { id: String, name: String },
    Duplicate { id: String },
    AskDelete { id: String, name: String },
    Delete { id: String },
    Move { from_collection: String, item_id: String, to_collection: String, container: String, index: Option<usize> },
    ExportCollection(String),
    ExportEnvironment(String),
    OpenRunner { collection_id: String, folder_id: Option<String> },
    ItemSettings { collection_id: String, folder_id: Option<String> },
    OpenImport,
    ImportFiles,
    ImportText(String),
    OpenSettings,
    OpenEnvs,
    SetEnv(Option<String>),
    ToggleTheme,
    OpenPalette,
    NewFlow,
    OpenFlow(String),
    RunFlow(String),
    StopFlow(String),
    SetBearer { collection_id: String, request_id: String, variable: String },
}

enum SaveMsg {
    Workspace(Box<Workspace>),
    Ui(UiState),
}

pub struct SwiftApp {
    pub ws: Workspace,
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_tab_id: u64,
    pub bridge: Bridge,
    storage: Arc<Storage>,
    saver: Sender<SaveMsg>,
    dirty: bool,
    last_change: Instant,
    last_ui_save: Instant,
    pub sidebar_mode: SidebarMode,
    pub sidebar_filter: String,
    pub expanded: HashSet<String>,
    pub renaming: Option<(String, String)>,
    pub actions: Vec<Action>,
    pub show_envs: bool,
    pub env_selected: Option<String>,
    pub show_settings: bool,
    pub show_import: bool,
    pub import_text: String,
    pub snippet: Option<(u64, SnippetLang)>,
    pub item_settings: Option<ItemSettings>,
    pub item_oauth_busy: bool,
    pub item_oauth_msg: Option<Result<String, String>>,
    pub save_as: Option<SaveAs>,
    pub confirm_delete: Option<(String, String)>,
    pub runner: RunnerState,
    pub palette: Palette,
    pub toasts: Vec<Toast>,
    pub flow_views: std::collections::HashMap<String, crate::ui::flow::FlowView>,
    applied_theme: Option<(bool, f32)>,
}

impl SwiftApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let root = Storage::default_root();
        Storage::migrate_legacy_data(&root);
        let storage = Arc::new(Storage::new(root));
        let ws = storage.load();
        let ui_state: UiState = storage.load_ui();

        let (tx, rx) = std::sync::mpsc::channel::<SaveMsg>();
        let st = storage.clone();
        std::thread::Builder::new()
            .name("tessera-saver".into())
            .spawn(move || {
                while let Ok(msg) = rx.recv() {
                    // Coalesce bursts: only the newest snapshot of each kind matters.
                    let mut latest_ws = None;
                    let mut latest_ui = None;
                    let mut put = |m: SaveMsg| match m {
                        SaveMsg::Workspace(w) => latest_ws = Some(w),
                        SaveMsg::Ui(u) => latest_ui = Some(u),
                    };
                    put(msg);
                    while let Ok(m) = rx.try_recv() {
                        put(m);
                    }
                    if let Some(w) = latest_ws {
                        if let Err(e) = st.save(&w) {
                            eprintln!("tessera: save failed: {e}");
                        }
                    }
                    if let Some(u) = latest_ui {
                        let _ = st.save_ui(&u);
                    }
                }
            })
            .expect("saver thread");

        let mut app = Self {
            ws,
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 1,
            bridge: Bridge::new(cc.egui_ctx.clone()),
            storage,
            saver: tx,
            dirty: false,
            last_change: Instant::now(),
            last_ui_save: Instant::now(),
            sidebar_mode: SidebarMode::Collections,
            sidebar_filter: String::new(),
            expanded: ui_state.expanded.iter().cloned().collect(),
            renaming: None,
            actions: Vec::new(),
            show_envs: false,
            env_selected: None,
            show_settings: false,
            show_import: false,
            import_text: String::new(),
            snippet: None,
            item_settings: None,
            item_oauth_busy: false,
            item_oauth_msg: None,
            save_as: None,
            confirm_delete: None,
            runner: RunnerState::default(),
            palette: Palette::default(),
            toasts: Vec::new(),
            flow_views: Default::default(),
            applied_theme: None,
        };
        for pt in ui_state.tabs {
            if let Some(fid) = pt.flow_id {
                if app.ws.flows.iter().any(|f| f.id == fid) {
                    let mut tab = Tab::new(app.alloc_tab_id(), Request::default(), None, false);
                    tab.flow_id = Some(fid);
                    app.tabs.push(tab);
                }
            } else if let Some(req) = pt.request {
                let mut tab = Tab::new(app.alloc_tab_id(), req, pt.collection_id, false);
                tab.saved = pt.saved;
                tab.req_section = pt.req_section;
                app.tabs.push(tab);
            }
        }
        app.active = ui_state.active.min(app.tabs.len().saturating_sub(1));
        if app.tabs.is_empty() {
            app.new_tab(Request::new(RequestKind::Http), None, false);
        }
        app
    }

    fn alloc_tab_id(&mut self) -> u64 {
        self.next_tab_id += 1;
        self.next_tab_id
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_change = Instant::now();
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, until: Instant::now() + Duration::from_secs(if error { 6 } else { 3 }) });
    }

    pub fn new_tab(&mut self, req: Request, collection_id: Option<String>, saved: bool) -> u64 {
        let id = self.alloc_tab_id();
        self.tabs.push(Tab::new(id, req, collection_id, saved));
        self.active = self.tabs.len() - 1;
        id
    }

    pub fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn ui_state(&self) -> UiState {
        UiState {
            tabs: self
                .tabs
                .iter()
                .map(|t| PersistedTab {
                    flow_id: t.flow_id.clone(),
                    collection_id: t.collection_id.clone(),
                    request: Some(t.request.clone()),
                    saved: t.saved.clone(),
                    req_section: t.req_section,
                })
                .collect(),
            active: self.active,
            expanded: self.expanded.iter().cloned().collect(),
        }
    }

    // ------------------------------------------------------------------ variables

    pub fn env_map(&self) -> BTreeMap<String, String> {
        self.ws.active_environment().map(|e| vars_to_map(&e.variables)).unwrap_or_default()
    }

    pub fn scope_for(&self, collection_id: Option<&str>) -> VarScope {
        VarScope {
            environment: self.env_map(),
            collection: collection_id.and_then(|c| self.ws.collection(c)).map(|c| vars_to_map(&c.variables)).unwrap_or_default(),
            globals: vars_to_map(&self.ws.globals),
            ..Default::default()
        }
    }

    /// Effective auth and parent scripts for a tab's request.
    fn tab_context(&self, tab: &Tab) -> (Auth, Vec<Scripts>) {
        match tab.collection_id.as_deref().and_then(|c| self.ws.collection(c)) {
            Some(c) if c.find_request(&tab.request.id).is_some() => (c.effective_auth(&tab.request), c.parent_scripts(&tab.request.id)),
            Some(c) => {
                let auth = if tab.request.auth.kind == AuthKind::Inherit { c.auth.clone() } else { tab.request.auth.clone() };
                (auth, vec![c.scripts.clone()])
            }
            None => {
                let auth = if tab.request.auth.kind == AuthKind::Inherit { Auth::none() } else { tab.request.auth.clone() };
                (auth, Vec::new())
            }
        }
    }

    /// Write variables changed by scripts back into the workspace.
    fn apply_script_vars(&mut self, collection_id: Option<&str>, vars: &ScriptVars) {
        let mut changed = false;
        if let Some(env) = self.ws.active_environment_mut() {
            if vars_to_map(&env.variables) != vars.env {
                merge_vars(&mut env.variables, &vars.env);
                changed = true;
            }
        }
        if let Some(c) = collection_id.and_then(|id| self.ws.collection_mut(id)) {
            if vars_to_map(&c.variables) != vars.collection {
                merge_vars(&mut c.variables, &vars.collection);
                changed = true;
            }
        }
        if vars_to_map(&self.ws.globals) != vars.globals {
            merge_vars(&mut self.ws.globals, &vars.globals);
            changed = true;
        }
        if changed {
            self.mark_dirty();
        }
    }

    fn script_vars(&self, collection_id: Option<&str>) -> ScriptVars {
        let scope = self.scope_for(collection_id);
        ScriptVars { env: scope.environment, collection: scope.collection, globals: scope.globals, locals: BTreeMap::new() }
    }

    // ------------------------------------------------------------------ sending

    fn send_tab(&mut self, tab_id: u64) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        if self.tabs[idx].request.kind == RequestKind::WebSocket {
            return self.toggle_ws(idx);
        }
        if self.tabs[idx].pending.is_some() {
            return;
        }
        let tab = &self.tabs[idx];
        let (auth, parent_scripts) = self.tab_context(tab);
        let ctx = SendContext {
            request: tab.request.clone(),
            auth,
            parent_scripts,
            vars: self.script_vars(tab.collection_id.as_deref()),
            data: BTreeMap::new(),
            env_name: self.ws.active_environment().map(|e| e.name.clone()).unwrap_or_default(),
            settings: self.ws.settings.clone(),
            certs: self.ws.certificates.clone(),
            info: ScriptInfo { request_name: tab.request.name.clone(), request_id: tab.request.id.clone(), iteration_count: 1, ..Default::default() },
        };
        let engine = self.bridge.engine.clone();
        let emitter = self.bridge.emitter();
        let handle = self.bridge.spawn(async move {
            let outcome = send(&engine, ctx).await;
            let view = match &outcome.response {
                Ok(r) => {
                    let r = r.clone();
                    tokio::task::spawn_blocking(move || crate::view::build(&r, tab_id)).await.unwrap_or_else(|_| crate::view::BodyView::empty())
                }
                Err(_) => crate::view::BodyView::empty(),
            };
            emitter.emit(AppEvent::Http { tab_id, outcome: Box::new(outcome), view: Box::new(view) });
        });
        self.tabs[idx].pending = Some(Pending { handle, started: Instant::now() });
    }

    fn toggle_ws(&mut self, idx: usize) {
        let tab_id = self.tabs[idx].id;
        if self.tabs[idx].ws.connected || self.tabs[idx].ws.connecting {
            let ws = &mut self.tabs[idx].ws;
            if let Some(tx) = ws.tx.take() {
                let _ = tx.send(WsCommand::Close);
            } else if let Some(h) = ws.handle.take() {
                h.abort();
            }
            ws.connecting = false;
            ws.connected = false;
            return;
        }
        let tab = &self.tabs[idx];
        let (auth, _) = self.tab_context(tab);
        let scope = self.scope_for(tab.collection_id.as_deref());
        let mut req = tab.request.clone();
        if !req.url.contains("://") && !req.url.trim().is_empty() {
            req.url = format!("ws://{}", req.url.trim());
        }
        let prepared = tessera_core::http::prepare(&req, &auth, &scope, &self.ws.settings);
        let headers: Vec<(String, String)> = prepared
            .headers
            .into_iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("accept"))
            .collect();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let emitter = self.bridge.emitter();
        let url = prepared.url.clone();
        let verify = prepared.verify_tls;
        let handle = self.bridge.spawn(async move {
            tessera_core::ws::run_session(url, headers, verify, rx, |event| {
                emitter.emit(AppEvent::Ws { tab_id, event });
            })
            .await;
        });
        let ws = &mut self.tabs[idx].ws;
        ws.tx = Some(tx);
        ws.handle = Some(handle);
        ws.connecting = true;
        ws.log.push(WsLine { incoming: false, system: true, text: format!("Connecting to {}", prepared.url), at: now_hms() });
    }

    fn ws_send(&mut self, tab_id: u64) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        let scope = self.scope_for(self.tabs[idx].collection_id.as_deref());
        let tab = &mut self.tabs[idx];
        let text = scope.resolve(&tab.ws.compose);
        if let Some(tx) = &tab.ws.tx {
            let _ = tx.send(WsCommand::Send(text));
        }
    }

    fn fetch_schema(&mut self, tab_id: u64) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        let tab = &self.tabs[idx];
        let (auth, _) = self.tab_context(tab);
        let scope = self.scope_for(tab.collection_id.as_deref());
        let mut req = tab.request.clone();
        req.kind = RequestKind::GraphQL;
        req.body.graphql_query = tessera_core::graphql::INTROSPECTION_QUERY.into();
        req.body.graphql_variables.clear();
        let prepared = tessera_core::http::prepare(&req, &auth, &scope, &self.ws.settings);
        let settings = self.ws.settings.clone();
        let certs = self.ws.certificates.clone();
        let engine = self.bridge.engine.clone();
        let emitter = self.bridge.emitter();
        self.tabs[idx].schema_loading = true;
        self.bridge.spawn(async move {
            let result = match engine.execute(&prepared, &settings, &certs).await {
                Ok(r) => tessera_core::graphql::parse_introspection(&r.text()),
                Err(e) => Err(e),
            };
            emitter.emit(AppEvent::Schema { tab_id, result });
        });
    }

    fn auth_of(&mut self, owner: &AuthOwner) -> Option<&mut Auth> {
        match owner {
            AuthOwner::Tab(id) => {
                let idx = self.tab_index(*id)?;
                Some(&mut self.tabs[idx].request.auth)
            }
            AuthOwner::Collection(cid) => self.ws.collection_mut(cid).map(|c| &mut c.auth),
            AuthOwner::Folder { collection, folder } => self.ws.collection_mut(collection)?.find_folder_mut(folder).map(|f| &mut f.auth),
        }
    }

    fn fetch_token(&mut self, owner: AuthOwner, refresh: bool) {
        let collection_id = match &owner {
            AuthOwner::Tab(id) => self.tab_index(*id).and_then(|i| self.tabs[i].collection_id.clone()),
            AuthOwner::Collection(c) | AuthOwner::Folder { collection: c, .. } => Some(c.clone()),
        };
        let scope = self.scope_for(collection_id.as_deref());
        let Some(cfg) = self.auth_of(&owner).map(|a| a.oauth2.clone()) else { return };
        match &owner {
            AuthOwner::Tab(id) => {
                if let Some(i) = self.tab_index(*id) {
                    self.tabs[i].oauth_busy = true;
                    self.tabs[i].oauth_msg = None;
                }
            }
            _ => {
                self.item_oauth_busy = true;
                self.item_oauth_msg = None;
            }
        }
        let emitter = self.bridge.emitter();
        self.bridge.spawn(async move {
            let result = if refresh {
                tessera_core::oauth::refresh_token(&cfg, &scope).await
            } else {
                tessera_core::oauth::fetch_token(&cfg, &scope).await
            };
            emitter.emit(AppEvent::OAuth { owner, result });
        });
    }

    // ------------------------------------------------------------------ events

    fn handle_events(&mut self) {
        for ev in self.bridge.drain() {
            match ev {
                AppEvent::Http { tab_id, outcome, view } => {
                    let Some(idx) = self.tab_index(tab_id) else { continue };
                    let collection_id = self.tabs[idx].collection_id.clone();
                    self.apply_script_vars(collection_id.as_deref(), &outcome.vars);
                    let tab = &mut self.tabs[idx];
                    tab.pending = None;
                    let (status, time) = match &outcome.response {
                        Ok(r) => (r.status, r.time_ms),
                        Err(_) => (0, 0.0),
                    };
                    tab.response = Some(ResponseState {
                        result: outcome.response,
                        prepared: outcome.prepared,
                        tests: outcome.tests,
                        console: outcome.console,
                        view: *view,
                        search: String::new(),
                        search_hit: 0,
                        scroll_to: None,
                    });
                    let req = tab.request.clone();
                    self.ws.push_history(req, status, time);
                    self.mark_dirty();
                }
                AppEvent::Ws { tab_id, event } => {
                    let Some(idx) = self.tab_index(tab_id) else { continue };
                    let ws = &mut self.tabs[idx].ws;
                    match event {
                        WsEvent::Connected { status, .. } => {
                            ws.connected = true;
                            ws.connecting = false;
                            ws.log.push(WsLine { incoming: true, system: true, text: format!("Connected ({status} Switching Protocols)"), at: now_hms() });
                        }
                        WsEvent::Message { incoming, text, at, .. } => ws.log.push(WsLine { incoming, system: false, text, at }),
                        WsEvent::Error(e) => {
                            if !ws.connected {
                                ws.connecting = false;
                                ws.tx = None;
                            }
                            ws.log.push(WsLine { incoming: true, system: true, text: format!("Error: {e}"), at: now_hms() });
                        }
                        WsEvent::Closed(reason) => {
                            ws.connected = false;
                            ws.connecting = false;
                            ws.tx = None;
                            ws.log.push(WsLine { incoming: true, system: true, text: reason, at: now_hms() });
                        }
                    }
                    if ws.log.len() > 5000 {
                        ws.log.drain(..1000);
                    }
                }
                AppEvent::Schema { tab_id, result } => {
                    if let Some(idx) = self.tab_index(tab_id) {
                        let ok = result.is_ok();
                        self.tabs[idx].schema = Some(result);
                        self.tabs[idx].schema_loading = false;
                        self.toast(if ok { "Schema loaded" } else { "Schema fetch failed" }, !ok);
                    }
                }
                AppEvent::OAuth { owner, result } => {
                    let msg = match &result {
                        Ok(t) => Ok(format!("Token received{}", if t.refresh_token.is_empty() { "" } else { " (with refresh token)" })),
                        Err(e) => Err(e.clone()),
                    };
                    if let Ok(tok) = &result {
                        if let Some(auth) = self.auth_of(&owner) {
                            auth.oauth2.access_token = tok.access_token.clone();
                            if !tok.refresh_token.is_empty() {
                                auth.oauth2.refresh_token = tok.refresh_token.clone();
                            }
                            auth.oauth2.expires_at = tok.expires_at;
                        }
                        self.mark_dirty();
                    }
                    match owner {
                        AuthOwner::Tab(id) => {
                            if let Some(i) = self.tab_index(id) {
                                self.tabs[i].oauth_busy = false;
                                self.tabs[i].oauth_msg = Some(msg);
                            }
                        }
                        _ => {
                            self.item_oauth_busy = false;
                            self.item_oauth_msg = Some(msg);
                        }
                    }
                }
                AppEvent::Run(ev) => self.handle_run_event(ev),
                AppEvent::Flow { flow_id, event } => self.handle_flow_event(flow_id, event),
            }
        }
    }

    fn handle_run_event(&mut self, ev: tessera_core::runner::RunEvent) {
        use tessera_core::runner::RunEvent;
        match ev {
            RunEvent::Started { total } => self.runner.total = total,
            RunEvent::Result(r) => self.runner.results.push(r),
            RunEvent::Finished { vars, cancelled } => {
                self.runner.running = false;
                self.runner.elapsed_ms = self.runner.started.map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
                if self.runner.persist_vars {
                    let cid = self.runner.collection_id.clone();
                    self.apply_script_vars(cid.as_deref(), &vars);
                }
                let failed = self.runner.results.iter().filter(|r| r.error.is_some() || r.tests.iter().any(|t| !t.passed)).count();
                let text = if cancelled {
                    "Run stopped".to_string()
                } else if failed == 0 {
                    format!("Run finished: {} requests passed", self.runner.results.len())
                } else {
                    format!("Run finished: {failed} of {} failed", self.runner.results.len())
                };
                self.toast(text, failed > 0);
            }
        }
    }

    pub fn start_run(&mut self) {
        let Some(c) = self.runner.collection_id.as_deref().and_then(|id| self.ws.collection(id)).cloned() else { return };
        let cfg = tessera_core::runner::RunConfig {
            vars: self.script_vars(Some(&c.id)),
            collection: c,
            folder_id: self.runner.folder_id.clone(),
            iterations: self.runner.iterations,
            delay_ms: self.runner.delay_ms,
            data: self.runner.data.clone(),
            env_name: self.ws.active_environment().map(|e| e.name.clone()).unwrap_or_default(),
            settings: self.ws.settings.clone(),
            certs: self.ws.certificates.clone(),
            stop_on_error: self.runner.stop_on_error,
        };
        self.runner.results.clear();
        self.runner.selected = None;
        self.runner.running = true;
        self.runner.started = Some(Instant::now());
        self.runner.cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = self.runner.cancel.clone();
        let engine = self.bridge.engine.clone();
        let emitter = self.bridge.emitter();
        self.bridge.spawn(async move {
            tessera_core::runner::run(engine, cfg, cancel, |ev| emitter.emit(AppEvent::Run(ev))).await;
        });
    }

    fn run_flow(&mut self, fid: &str) {
        let Some(flow) = self.ws.flows.iter().find(|f| f.id == fid).cloned() else { return };
        if flow.nodes.is_empty() {
            return self.toast("Add some requests to the flow first", true);
        }
        let scope = self.scope_for(None);
        let cfg = tessera_core::flow::FlowRunConfig {
            flow,
            collections: self.ws.collections.clone(),
            vars: ScriptVars { env: scope.environment, globals: scope.globals, ..Default::default() },
            env_name: self.ws.active_environment().map(|e| e.name.clone()).unwrap_or_default(),
            settings: self.ws.settings.clone(),
            certs: self.ws.certificates.clone(),
        };
        let view = self.flow_views.entry(fid.to_string()).or_default();
        view.reset_run();
        let cancel = view.run.cancel.clone();
        let engine = self.bridge.engine.clone();
        let emitter = self.bridge.emitter();
        let flow_id = fid.to_string();
        self.bridge.spawn(async move {
            tessera_core::flow::run_flow(engine, cfg, cancel, |event| emitter.emit(AppEvent::Flow { flow_id: flow_id.clone(), event })).await;
        });
    }

    fn handle_flow_event(&mut self, flow_id: String, event: tessera_core::flow::FlowEvent) {
        use tessera_core::flow::{FlowEvent, NodeResult, NodeStatus};
        match event {
            FlowEvent::NodeStarted(node) => {
                self.flow_views.entry(flow_id).or_default().run.results.insert(node, NodeResult::running());
            }
            FlowEvent::NodeFinished(node, result) => {
                if let Some(resp) = &result.response {
                    let req = self
                        .ws
                        .flows
                        .iter()
                        .find(|f| f.id == flow_id)
                        .and_then(|f| f.node(&node))
                        .and_then(|n| self.ws.collection(&n.collection_id).and_then(|c| c.find_request(&n.request_id)))
                        .cloned();
                    if let Some(req) = req {
                        self.ws.push_history(req, resp.status, resp.time_ms);
                    }
                }
                self.flow_views.entry(flow_id).or_default().run.results.insert(node, *result);
            }
            FlowEvent::Finished { vars, collections, flow_vars, error, cancelled } => {
                let view = self.flow_views.entry(flow_id).or_default();
                view.run.running = false;
                view.run.elapsed_ms = view.run.started.map(|s| s.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
                view.run.flow_vars = flow_vars;
                let failed = view.run.results.values().filter(|r| r.status == NodeStatus::Failed).count();
                let passed = view.run.results.values().filter(|r| r.status == NodeStatus::Passed).count();
                // Persist variables changed by the requests' scripts or by "save to env" wires.
                for (cid, map) in collections {
                    if let Some(c) = self.ws.collection_mut(&cid) {
                        if vars_to_map(&c.variables) != map {
                            merge_vars(&mut c.variables, &map);
                        }
                    }
                }
                if let Some(env) = self.ws.active_environment_mut() {
                    if vars_to_map(&env.variables) != vars.env {
                        merge_vars(&mut env.variables, &vars.env);
                    }
                }
                if vars_to_map(&self.ws.globals) != vars.globals {
                    merge_vars(&mut self.ws.globals, &vars.globals);
                }
                self.mark_dirty();
                match (error, cancelled) {
                    (Some(e), _) => self.toast(e, true),
                    (None, true) => self.toast("Flow stopped", false),
                    (None, false) if failed == 0 => self.toast(format!("Flow passed: {passed} steps"), false),
                    (None, false) => self.toast(format!("Flow finished: {failed} step(s) failed"), true),
                }
            }
        }
    }

    pub fn stop_run(&mut self) {
        self.runner.cancel.store(true, Ordering::Relaxed);
    }

    // ------------------------------------------------------------------ import / export

    pub fn import_text(&mut self, text: &str) {
        match postman::import_any(text) {
            Ok(items) => {
                let mut summary = Vec::new();
                for it in items {
                    match it {
                        Imported::Collection(c) => {
                            summary.push(format!("collection '{}' ({} requests)", c.name, c.all_requests().len()));
                            self.expanded.insert(c.id.clone());
                            self.ws.collections.push(c);
                        }
                        Imported::Environment(e) => {
                            summary.push(format!("environment '{}'", e.name));
                            if self.ws.active_env.is_none() {
                                self.ws.active_env = Some(e.id.clone());
                            }
                            self.ws.environments.push(e);
                        }
                        Imported::Request(r) => {
                            summary.push(format!("request '{}'", r.name));
                            self.new_tab(r, None, false);
                        }
                    }
                }
                self.mark_dirty();
                self.toast(format!("Imported {}", summary.join(", ")), false);
                self.show_import = false;
                self.import_text.clear();
            }
            Err(e) => self.toast(e, true),
        }
    }

    fn import_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        for p in paths {
            match std::fs::read_to_string(&p) {
                Ok(text) => self.import_text(&text),
                Err(e) => self.toast(format!("{}: {e}", p.display()), true),
            }
        }
    }

    // ------------------------------------------------------------------ actions

    fn process_actions(&mut self) {
        let actions = std::mem::take(&mut self.actions);
        for a in actions {
            self.process(a);
        }
    }

    fn process(&mut self, a: Action) {
        match a {
            Action::Send(id) => self.send_tab(id),
            Action::NewFlow => {
                let f = tessera_core::flow::Flow::new(format!("Flow {}", self.ws.flows.len() + 1));
                let id = f.id.clone();
                self.ws.flows.push(f);
                self.sidebar_mode = SidebarMode::Flows;
                self.mark_dirty();
                self.process(Action::OpenFlow(id));
            }
            Action::OpenFlow(fid) => {
                if let Some(i) = self.tabs.iter().position(|t| t.flow_id.as_deref() == Some(fid.as_str())) {
                    self.active = i;
                } else if self.ws.flows.iter().any(|f| f.id == fid) {
                    let id = self.new_tab(Request::default(), None, false);
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].flow_id = Some(fid);
                    }
                }
            }
            Action::RunFlow(fid) => self.run_flow(&fid),
            Action::StopFlow(fid) => {
                if let Some(v) = self.flow_views.get(&fid) {
                    v.run.cancel.store(true, Ordering::Relaxed);
                }
            }
            Action::SetBearer { collection_id, request_id, variable } => {
                let token = format!("{{{{{variable}}}}}");
                if let Some(r) = self.ws.collection_mut(&collection_id).and_then(|c| c.find_request_mut(&request_id)) {
                    r.auth.kind = AuthKind::Bearer;
                    r.auth.bearer_token = token.clone();
                    let saved = r.clone();
                    for t in self.tabs.iter_mut().filter(|t| t.request.id == request_id) {
                        t.request.auth = saved.auth.clone();
                        t.saved = Some(saved.clone());
                    }
                    self.mark_dirty();
                    self.toast(format!("“{}” now sends Authorization: Bearer {token}", saved.name), false);
                }
            }
            Action::Cancel(id) => {
                if let Some(i) = self.tab_index(id) {
                    if let Some(p) = self.tabs[i].pending.take() {
                        p.handle.abort();
                        self.toast("Request cancelled", false);
                    }
                }
            }
            Action::Save(id) => self.save_tab(id),
            Action::Snippet(id) => self.snippet = Some((id, SnippetLang::Curl)),
            Action::FetchSchema(id) => self.fetch_schema(id),
            Action::WsSend(id) => self.ws_send(id),
            Action::FetchToken { owner, refresh } => self.fetch_token(owner, refresh),
            Action::Toast(t, e) => self.toast(t, e),
            Action::NewTab(kind) => {
                self.new_tab(Request::new(kind), None, false);
            }
            Action::CloseTab(id) => {
                if let Some(i) = self.tab_index(id) {
                    let tab = self.tabs.remove(i);
                    if let Some(p) = tab.pending {
                        p.handle.abort();
                    }
                    if let Some(tx) = tab.ws.tx {
                        let _ = tx.send(WsCommand::Close);
                    }
                    if self.active >= i && self.active > 0 {
                        self.active -= 1;
                    }
                    if self.tabs.is_empty() {
                        self.active = 0;
                    }
                }
            }
            Action::CloseOtherTabs(id) => {
                self.tabs.retain(|t| t.id == id);
                self.active = 0;
            }
            Action::DuplicateTab(id) => {
                if let Some(i) = self.tab_index(id) {
                    let mut req = self.tabs[i].request.clone();
                    req.id = new_id();
                    req.name = format!("{} Copy", req.name);
                    self.new_tab(req, None, false);
                }
            }
            Action::OpenRequest { collection_id, request_id } => {
                if let Some(i) = self.tabs.iter().position(|t| t.request.id == request_id) {
                    self.active = i;
                } else if let Some(r) = self.ws.collection(&collection_id).and_then(|c| c.find_request(&request_id)).cloned() {
                    self.new_tab(r, Some(collection_id), true);
                }
            }
            Action::OpenHistory(i) => {
                if let Some(e) = self.ws.history.get(i) {
                    let mut r = e.request.clone();
                    r.id = new_id();
                    self.new_tab(r, None, false);
                }
            }
            Action::DeleteHistory(i) => {
                if i < self.ws.history.len() {
                    self.ws.history.remove(i);
                    self.mark_dirty();
                }
            }
            Action::ClearHistory => {
                self.ws.history.clear();
                self.mark_dirty();
            }
            Action::NewCollection => {
                let c = Collection::new("New Collection");
                self.expanded.insert(c.id.clone());
                self.renaming = Some((c.id.clone(), c.name.clone()));
                self.ws.collections.push(c);
                self.sidebar_mode = SidebarMode::Collections;
                self.mark_dirty();
            }
            Action::AddRequest { collection_id, container, kind } => {
                let req = Request::new(kind);
                if let Some(list) = self.ws.collection_mut(&collection_id).and_then(|c| c.container_mut(&container)) {
                    list.push(Item::Request(req.clone()));
                    self.expanded.insert(container);
                    self.expanded.insert(collection_id.clone());
                    self.new_tab(req, Some(collection_id), true);
                    self.mark_dirty();
                }
            }
            Action::AddFolder { collection_id, container } => {
                let f = Folder::new("New Folder");
                let fid = f.id.clone();
                if let Some(list) = self.ws.collection_mut(&collection_id).and_then(|c| c.container_mut(&container)) {
                    list.push(Item::Folder(f));
                    self.expanded.insert(container);
                    self.expanded.insert(fid.clone());
                    self.renaming = Some((fid, "New Folder".into()));
                    self.mark_dirty();
                }
            }
            Action::Rename { id, name } => {
                if let Some(f) = self.ws.flows.iter_mut().find(|f| f.id == id) {
                    f.name = name;
                } else if let Some(c) = self.ws.collection_mut(&id) {
                    c.name = name;
                } else if let Some(c) = self.ws.collection_of_mut(&id) {
                    if let Some(f) = c.find_folder_mut(&id) {
                        f.name = name;
                    } else if let Some(r) = c.find_request_mut(&id) {
                        r.name = name.clone();
                        for t in self.tabs.iter_mut().filter(|t| t.request.id == id) {
                            t.request.name = name.clone();
                            if let Some(s) = t.saved.as_mut() {
                                s.name = name.clone();
                            }
                        }
                    }
                }
                self.mark_dirty();
            }
            Action::Duplicate { id } => {
                if let Some(pos) = self.ws.flows.iter().position(|f| f.id == id) {
                    let mut copy = self.ws.flows[pos].clone();
                    copy.id = new_id();
                    copy.name = format!("{} Copy", copy.name);
                    self.ws.flows.insert(pos + 1, copy);
                } else if let Some(pos) = self.ws.collections.iter().position(|c| c.id == id) {
                    let src = &self.ws.collections[pos];
                    let mut copy = Collection::new(format!("{} Copy", src.name));
                    copy.items = src.items.iter().map(|i| i.with_new_ids()).collect();
                    copy.variables = src.variables.clone();
                    copy.auth = src.auth.clone();
                    copy.scripts = src.scripts.clone();
                    copy.description = src.description.clone();
                    self.ws.collections.insert(pos + 1, copy);
                } else if let Some(c) = self.ws.collection_of_mut(&id) {
                    if let Some((container, idx)) = c.locate(&id) {
                        let list = c.container_mut(&container).unwrap();
                        let mut copy = list[idx].with_new_ids();
                        match &mut copy {
                            Item::Folder(f) => f.name = format!("{} Copy", f.name),
                            Item::Request(r) => r.name = format!("{} Copy", r.name),
                        }
                        list.insert(idx + 1, copy);
                    }
                }
                self.mark_dirty();
            }
            Action::AskDelete { id, name } => self.confirm_delete = Some((id, name)),
            Action::Delete { id } => {
                if let Some(pos) = self.ws.flows.iter().position(|f| f.id == id) {
                    self.ws.flows.remove(pos);
                    self.flow_views.remove(&id);
                    self.tabs.retain(|t| t.flow_id.as_deref() != Some(id.as_str()));
                    self.active = self.active.min(self.tabs.len().saturating_sub(1));
                } else if let Some(pos) = self.ws.collections.iter().position(|c| c.id == id) {
                    self.ws.collections.remove(pos);
                } else if let Some(c) = self.ws.collection_of_mut(&id) {
                    c.remove_item(&id);
                }
                self.relink_tabs();
                self.mark_dirty();
            }
            Action::Move { from_collection, item_id, to_collection, container, index } => {
                self.move_item(&from_collection, &item_id, &to_collection, &container, index);
            }
            Action::ExportCollection(id) => {
                if let Some(c) = self.ws.collection(&id) {
                    let json = serde_json::to_string_pretty(&postman::export_collection(c)).unwrap_or_default();
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(format!("{}.postman_collection.json", sanitize(&c.name)))
                        .add_filter("JSON", &["json"])
                        .save_file()
                    {
                        match std::fs::write(&path, json) {
                            Ok(_) => self.toast(format!("Exported to {}", path.display()), false),
                            Err(e) => self.toast(format!("Export failed: {e}"), true),
                        }
                    }
                }
            }
            Action::ExportEnvironment(id) => {
                if let Some(e) = self.ws.environments.iter().find(|e| e.id == id) {
                    let json = serde_json::to_string_pretty(&postman::export_environment(e)).unwrap_or_default();
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(format!("{}.postman_environment.json", sanitize(&e.name)))
                        .add_filter("JSON", &["json"])
                        .save_file()
                    {
                        match std::fs::write(&path, json) {
                            Ok(_) => self.toast(format!("Exported to {}", path.display()), false),
                            Err(e) => self.toast(format!("Export failed: {e}"), true),
                        }
                    }
                }
            }
            Action::OpenRunner { collection_id, folder_id } => {
                if !self.runner.running {
                    if self.runner.collection_id.as_ref() != Some(&collection_id) || self.runner.folder_id != folder_id {
                        self.runner.results.clear();
                    }
                    self.runner.collection_id = Some(collection_id);
                    self.runner.folder_id = folder_id;
                }
                self.runner.open = true;
            }
            Action::ItemSettings { collection_id, folder_id } => {
                self.item_oauth_msg = None;
                self.item_settings = Some(ItemSettings { collection_id, folder_id, section: ItemSection::Overview });
            }
            Action::OpenImport => self.show_import = true,
            Action::ImportFiles => {
                if let Some(paths) = rfd::FileDialog::new().add_filter("Postman / JSON", &["json"]).add_filter("All files", &["*"]).pick_files() {
                    self.import_paths(paths);
                }
            }
            Action::ImportText(t) => self.import_text(&t),
            Action::OpenSettings => self.show_settings = true,
            Action::OpenEnvs => {
                self.env_selected = self.ws.active_env.clone();
                self.show_envs = true;
            }
            Action::SetEnv(id) => {
                self.ws.active_env = id;
                self.mark_dirty();
            }
            Action::ToggleTheme => {
                self.ws.settings.dark_mode = !self.ws.settings.dark_mode;
                self.mark_dirty();
            }
            Action::OpenPalette => {
                self.palette.open = true;
                self.palette.query.clear();
                self.palette.selected = 0;
                self.palette.focus_pending = true;
            }
        }
    }

    fn move_item(&mut self, from: &str, item_id: &str, to: &str, container: &str, index: Option<usize>) {
        // Never move a folder into itself or its descendants.
        let Some(src) = self.ws.collection(from) else { return };
        let Some((src_container, src_idx)) = src.locate(item_id) else { return };
        let item_contains_target = {
            let mut c = src.clone();
            c.remove_item(item_id).is_some_and(|it| it.contains_id(container))
        };
        if item_contains_target {
            return;
        }
        let Some(item) = self.ws.collection_mut(from).and_then(|c| c.remove_item(item_id)) else { return };
        let Some(list) = self.ws.collection_mut(to).and_then(|c| c.container_mut(container)) else {
            // Put it back if the target vanished.
            if let Some(l) = self.ws.collection_mut(from).and_then(|c| c.container_mut(&src_container)) {
                l.insert(src_idx.min(l.len()), item);
            }
            return;
        };
        let mut idx = index.unwrap_or(list.len());
        if from == to && src_container == container && src_idx < idx {
            idx -= 1;
        }
        list.insert(idx.min(list.len()), item);
        self.expanded.insert(container.to_string());
        self.relink_tabs();
        self.mark_dirty();
    }

    /// Re-point tabs at the collection that now holds their request (after moves / deletes).
    fn relink_tabs(&mut self) {
        for t in self.tabs.iter_mut() {
            if t.collection_id.is_some() || t.saved.is_some() {
                match self.ws.collections.iter().find(|c| c.find_request(&t.request.id).is_some()) {
                    Some(c) => t.collection_id = Some(c.id.clone()),
                    None => {
                        t.collection_id = None;
                        t.saved = None;
                    }
                }
            }
        }
    }

    fn save_tab(&mut self, tab_id: u64) {
        let Some(idx) = self.tab_index(tab_id) else { return };
        let req = self.tabs[idx].request.clone();
        let cid = self.tabs[idx].collection_id.clone();
        if let Some(target) = cid.as_deref().and_then(|c| self.ws.collection_mut(c)).and_then(|c| c.find_request_mut(&req.id)) {
            *target = req.clone();
            self.tabs[idx].saved = Some(req);
            self.mark_dirty();
            self.toast("Saved", false);
        } else {
            let collection_id = self.ws.collections.first().map(|c| c.id.clone());
            let name = if req.name.starts_with("New ") && !req.url.is_empty() {
                let (base, _) = tessera_core::vars::split_query(&req.url);
                base.rsplit('/').find(|s| !s.is_empty()).unwrap_or(&req.name).to_string()
            } else {
                req.name.clone()
            };
            self.save_as = Some(SaveAs { tab_id, name, collection_id, folder_id: None, new_collection: String::new() });
        }
    }

    pub fn commit_save_as(&mut self) {
        let Some(sa) = self.save_as.take() else { return };
        let Some(idx) = self.tab_index(sa.tab_id) else { return };
        let cid = match sa.collection_id {
            Some(c) => c,
            None => {
                let name = if sa.new_collection.trim().is_empty() { "My Collection".to_string() } else { sa.new_collection.trim().to_string() };
                let c = Collection::new(name);
                let id = c.id.clone();
                self.ws.collections.push(c);
                id
            }
        };
        let mut req = self.tabs[idx].request.clone();
        req.name = if sa.name.trim().is_empty() { "Untitled".into() } else { sa.name.trim().to_string() };
        // A fresh id so saving a history copy twice never aliases.
        if self.ws.collections.iter().any(|c| c.find_request(&req.id).is_some()) {
            req.id = new_id();
        }
        let container = sa.folder_id.clone().unwrap_or(cid.clone());
        if let Some(list) = self.ws.collection_mut(&cid).and_then(|c| c.container_mut(&container)) {
            list.push(Item::Request(req.clone()));
            self.expanded.insert(cid.clone());
            self.expanded.insert(container);
            let tab = &mut self.tabs[idx];
            tab.request = req.clone();
            tab.collection_id = Some(cid);
            tab.saved = Some(req);
            self.mark_dirty();
            self.toast("Saved to collection", false);
        }
    }

    // ------------------------------------------------------------------ input

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let active_id = self.tabs.get(self.active).map(|t| t.id);
        let (send, save, new, close, palette, focus_url, next_tab, prev_tab) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&cmd(Key::Enter)),
                i.consume_shortcut(&cmd(Key::S)),
                i.consume_shortcut(&cmd(Key::T)) || i.consume_shortcut(&cmd(Key::N)),
                i.consume_shortcut(&cmd(Key::W)),
                i.consume_shortcut(&cmd(Key::K)) || i.consume_shortcut(&cmd(Key::P)),
                i.consume_shortcut(&cmd(Key::L)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Tab)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab)),
            )
        });
        if let Some(id) = active_id {
            if send {
                let t = &self.tabs[self.active];
                if let Some(fid) = &t.flow_id {
                    let fid = fid.clone();
                    if !self.flow_views.get(&fid).is_some_and(|v| v.run.running) {
                        self.actions.push(Action::RunFlow(fid));
                    }
                } else if t.request.kind == RequestKind::WebSocket {
                    // Ctrl+Enter connects, then sends; it never tears down a connection.
                    if t.ws.connected {
                        self.actions.push(Action::WsSend(id));
                    } else if !t.ws.connecting {
                        self.actions.push(Action::Send(id));
                    }
                } else {
                    self.actions.push(Action::Send(id));
                }
            }
            if save && self.tabs[self.active].flow_id.is_none() {
                self.actions.push(Action::Save(id));
            }
            if close {
                self.actions.push(Action::CloseTab(id));
            }
            if focus_url {
                ctx.memory_mut(|m| m.request_focus(request::url_id(id)));
            }
        }
        if new {
            self.actions.push(Action::NewTab(RequestKind::Http));
        }
        if palette {
            self.actions.push(Action::OpenPalette);
        }
        if !self.tabs.is_empty() {
            if next_tab {
                self.active = (self.active + 1) % self.tabs.len();
            }
            if prev_tab {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) && !self.palette.open {
            if let Some(t) = self.tabs.get(self.active) {
                if t.pending.is_some() {
                    self.actions.push(Action::Cancel(t.id));
                }
            }
        }
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            self.import_paths(dropped);
        }
    }

    // ------------------------------------------------------------------ layout

    fn top_bar(&mut self, ctx: &egui::Context) {
        let p = theme::palette(self.ws.settings.dark_mode);
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::new().fill(theme::wordmark_bg(self.ws.settings.dark_mode)).inner_margin(egui::Margin::symmetric(12, 8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (uri, bytes) = if self.ws.settings.dark_mode {
                        ("bytes://tessera-wordmark-dark.png", theme::WORDMARK_DARK_PNG)
                    } else {
                        ("bytes://tessera-wordmark-light.png", theme::WORDMARK_LIGHT_PNG)
                    };
                    let h = 24.0;
                    ui.add(egui::Image::from_bytes(uri, bytes).fit_to_exact_size(egui::vec2(h * 329.0 / 72.0, h)));
                    ui.add_space(12.0);
                    ui.menu_button("➕ New", |ui| {
                        if ui.button("HTTP request").clicked() {
                            self.actions.push(Action::NewTab(RequestKind::Http));
                            ui.close_menu();
                        }
                        if ui.button("GraphQL request").clicked() {
                            self.actions.push(Action::NewTab(RequestKind::GraphQL));
                            ui.close_menu();
                        }
                        if ui.button("WebSocket").clicked() {
                            self.actions.push(Action::NewTab(RequestKind::WebSocket));
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Collection").clicked() {
                            self.actions.push(Action::NewCollection);
                            ui.close_menu();
                        }
                        if ui.button("Environment").clicked() {
                            let e = Environment::new("New Environment");
                            self.env_selected = Some(e.id.clone());
                            self.ws.environments.push(e);
                            self.show_envs = true;
                            self.mark_dirty();
                            ui.close_menu();
                        }
                    });
                    if ui.button("⤓ Import").clicked() {
                        self.actions.push(Action::OpenImport);
                    }
                    if ui.button("▶ Runner").clicked() {
                        if let Some(c) = self.runner.collection_id.clone().or_else(|| self.ws.collections.first().map(|c| c.id.clone())) {
                            let folder = self.runner.folder_id.clone();
                            self.actions.push(Action::OpenRunner { collection_id: c, folder_id: folder });
                        } else {
                            self.toast("Create or import a collection first", true);
                        }
                    }

                    // Centered search button.
                    let search_w = 300.0_f32.min(ui.available_width() - 330.0).max(120.0);
                    let space = ((ui.available_width() - search_w) / 2.0 - 170.0).max(8.0);
                    ui.add_space(space);
                    let search = egui::Button::new(RichText::new("🔍  Search requests & actions…        Ctrl+K").color(p.dim)).min_size(egui::vec2(search_w, 26.0)).fill(p.surface);
                    if ui.add(search).clicked() {
                        self.actions.push(Action::OpenPalette);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚙").on_hover_text("Settings").clicked() {
                            self.actions.push(Action::OpenSettings);
                        }
                        let icon = if self.ws.settings.dark_mode { "☀" } else { "🌙" };
                        if ui.button(icon).on_hover_text("Toggle theme").clicked() {
                            self.actions.push(Action::ToggleTheme);
                        }
                        if ui.button("👁").on_hover_text("Manage environments").clicked() {
                            self.actions.push(Action::OpenEnvs);
                        }
                        let current = self.ws.active_environment().map(|e| e.name.clone()).unwrap_or_else(|| "No environment".into());
                        let mut selected = self.ws.active_env.clone();
                        egui::ComboBox::from_id_salt("env")
                            .width(170.0)
                            .selected_text(RichText::new(current).color(if selected.is_some() { p.text } else { p.dim }))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut selected, None, "No environment");
                                for e in &self.ws.environments {
                                    ui.selectable_value(&mut selected, Some(e.id.clone()), &e.name);
                                }
                            });
                        if selected != self.ws.active_env {
                            self.actions.push(Action::SetEnv(selected));
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let p = theme::palette(self.ws.settings.dark_mode);
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::symmetric(12, 3)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let n_req: usize = self.ws.collections.iter().map(|c| c.all_requests().len()).sum();
                    ui.label(dim(ui, format!("{} collections · {} requests · {} environments", self.ws.collections.len(), n_req, self.ws.environments.len())).size(11.5));
                    if self.runner.running {
                        ui.separator();
                        ui.spinner();
                        ui.label(dim(ui, format!("Running {}/{}", self.runner.results.len(), self.runner.total)).size(11.5));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(dim(ui, concat!("v", env!("CARGO_PKG_VERSION"))).size(11.0));
                        if ui.add(egui::Button::new(dim(ui, "📂 Data folder").size(11.5)).frame(false)).on_hover_text(self.storage.root().display().to_string()).clicked() {
                            let _ = std::fs::create_dir_all(self.storage.root());
                            let _ = open::that(self.storage.root());
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = theme::palette(self.ws.settings.dark_mode);
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(280.0)
            .width_range(200.0..=520.0)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::same(10)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    widgets::segmented(ui, &mut self.sidebar_mode, &[(SidebarMode::Collections, "Collections"), (SidebarMode::Flows, "Flows"), (SidebarMode::History, "History")]);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match self.sidebar_mode {
                        SidebarMode::Collections => {
                            if ui.button("➕").on_hover_text("New collection").clicked() {
                                self.actions.push(Action::NewCollection);
                            }
                        }
                        SidebarMode::Flows => {
                            if ui.button("➕").on_hover_text("New flow").clicked() {
                                self.actions.push(Action::NewFlow);
                            }
                        }
                        SidebarMode::History => {
                            if !self.ws.history.is_empty() && ui.button("🗑").on_hover_text("Clear history").clicked() {
                                self.actions.push(Action::ClearHistory);
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                ui.add(egui::TextEdit::singleline(&mut self.sidebar_filter).hint_text("🔍 Filter").desired_width(f32::INFINITY));
                ui.add_space(6.0);
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match self.sidebar_mode {
                    SidebarMode::Collections => {
                        let active_request = self.tabs.get(self.active).filter(|t| t.collection_id.is_some()).map(|t| t.request.id.clone());
                        let mut sctx = SidebarCtx {
                            expanded: &mut self.expanded,
                            renaming: &mut self.renaming,
                            filter: &self.sidebar_filter,
                            active_request: active_request.as_deref(),
                            actions: &mut self.actions,
                        };
                        sidebar::collections(ui, &self.ws.collections, &mut sctx);
                    }
                    SidebarMode::Flows => {
                        let active_flow = self.tabs.get(self.active).and_then(|t| t.flow_id.clone());
                        let mut sctx = SidebarCtx {
                            expanded: &mut self.expanded,
                            renaming: &mut self.renaming,
                            filter: &self.sidebar_filter,
                            active_request: None,
                            actions: &mut self.actions,
                        };
                        sidebar::flows(ui, &self.ws.flows, &mut sctx, active_flow.as_deref());
                        ui.add_space(12.0);
                        ui.label(dim(ui, "Tip: switch to Collections and drag requests onto an open flow.").size(11.0));
                    }
                    SidebarMode::History => sidebar::history(ui, &self.ws.history, &self.sidebar_filter, &mut self.actions),
                });
            });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let dark = ui.visuals().dark_mode;
        ui.horizontal(|ui| {
            egui::ScrollArea::horizontal().id_salt("tabs").max_width(ui.available_width() - 40.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let selected = i == self.active;
                        let fill = if selected { p.surface } else { egui::Color32::TRANSPARENT };
                        let frame = egui::Frame::new()
                            .fill(fill)
                            .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                            .inner_margin(egui::Margin { left: 10, right: 6, top: 5, bottom: 5 })
                            .stroke(if selected { egui::Stroke::new(1.0_f32, p.border) } else { egui::Stroke::NONE });
                        let resp = frame
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let flow_name = tab.flow_id.as_ref().and_then(|fid| self.ws.flows.iter().find(|f| &f.id == fid)).map(|f| f.name.clone());
                                    let mut title = match &flow_name {
                                        Some(name) => {
                                            ui.label(RichText::new("FLOW").strong().size(10.5).color(theme::method_color("WS", dark)));
                                            name.clone()
                                        }
                                        None => {
                                            ui.label(widgets::method_text(tab.request.badge(), dark, 10.5));
                                            tab.title()
                                        }
                                    };
                                    if title.chars().count() > 26 {
                                        title = title.chars().take(25).collect::<String>() + "…";
                                    }
                                    ui.label(RichText::new(title).color(if selected { p.text } else { p.dim }));
                                    let dirty = tab.is_dirty();
                                    let close_text = if dirty { RichText::new("●").color(p.accent).size(11.0) } else { RichText::new("✕").color(p.dim).size(11.0) };
                                    let close = ui.add(egui::Button::new(close_text).frame(false).min_size(egui::vec2(16.0, 16.0)));
                                    if close.clicked() {
                                        self.actions.push(Action::CloseTab(tab.id));
                                    }
                                    if tab.pending.is_some() {
                                        ui.spinner();
                                    }
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        if resp.clicked() {
                            self.active = i;
                        }
                        if resp.middle_clicked() {
                            self.actions.push(Action::CloseTab(tab.id));
                        }
                        let id = tab.id;
                        resp.on_hover_text(if tab.request.url.is_empty() { tab.title() } else { tab.request.url.clone() }).context_menu(|ui| {
                            if ui.button("Duplicate tab").clicked() {
                                self.actions.push(Action::DuplicateTab(id));
                                ui.close_menu();
                            }
                            if ui.button("Close").clicked() {
                                self.actions.push(Action::CloseTab(id));
                                ui.close_menu();
                            }
                            if ui.button("Close others").clicked() {
                                self.actions.push(Action::CloseOtherTabs(id));
                                ui.close_menu();
                            }
                        });
                    }
                });
            });
            ui.menu_button("➕", |ui| {
                if ui.button("HTTP request").clicked() {
                    self.actions.push(Action::NewTab(RequestKind::Http));
                    ui.close_menu();
                }
                if ui.button("GraphQL").clicked() {
                    self.actions.push(Action::NewTab(RequestKind::GraphQL));
                    ui.close_menu();
                }
                if ui.button("WebSocket").clicked() {
                    self.actions.push(Action::NewTab(RequestKind::WebSocket));
                    ui.close_menu();
                }
            });
        });
        let rect = ui.min_rect();
        ui.painter().hline(rect.x_range(), rect.bottom() + 1.0, egui::Stroke::new(1.0_f32, p.border));
    }

    fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let p = theme::palette(self.ws.settings.dark_mode);
        let mut path: Vec<String> = Vec::new();
        if let Some(c) = tab.collection_id.as_deref().and_then(|id| self.ws.collections.iter().find(|c| c.id == id)) {
            path.push(c.name.clone());
            if let Some(chain) = c.folder_chain(&tab.request.id) {
                path.extend(chain.iter().map(|f| f.name.clone()));
            }
        }
        ui.horizontal(|ui| {
            for part in &path {
                ui.label(RichText::new(part).color(p.dim));
                ui.label(RichText::new("/").color(p.dim));
            }
            let w = (tab.request.name.len() as f32 * 8.0 + 30.0).clamp(120.0, 420.0);
            ui.add(egui::TextEdit::singleline(&mut tab.request.name).frame(false).desired_width(w).font(egui::TextStyle::Body));
            if tab.collection_id.is_none() {
                ui.label(RichText::new("· unsaved").color(p.dim).size(11.5));
            }
        });
    }

    fn central(&mut self, ctx: &egui::Context) {
        let p = theme::palette(self.ws.settings.dark_mode);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin { left: 10, right: 12, top: 6, bottom: 6 }))
            .show(ctx, |ui| {
                self.tab_strip(ui);
                if self.tabs.is_empty() {
                    widgets::logo_empty_state(ui, "No open tabs", "Ctrl+T for a new request · Ctrl+K to search");
                    ui.vertical_centered(|ui| {
                        if ui.button("New request").clicked() {
                            self.actions.push(Action::NewTab(RequestKind::Http));
                        }
                    });
                    return;
                }
                self.active = self.active.min(self.tabs.len() - 1);
                egui::Frame::new()
                    .fill(p.surface)
                    .corner_radius(egui::CornerRadius { nw: 0, ne: 8, sw: 8, se: 8 })
                    .stroke(egui::Stroke::new(1.0_f32, p.border))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        let idx = self.active;
                        if let Some(fid) = self.tabs[idx].flow_id.clone() {
                            let view = self.flow_views.entry(fid.clone()).or_default();
                            let ws = &mut self.ws;
                            let cols = &ws.collections;
                            if let Some(flow) = ws.flows.iter_mut().find(|f| f.id == fid) {
                                let mut actions = Vec::new();
                                if crate::ui::flow::show(ui, flow, view, cols, &mut actions) {
                                    self.dirty = true;
                                    self.last_change = Instant::now();
                                }
                                self.actions.extend(actions);
                            }
                            return;
                        }
                        self.breadcrumb(ui);
                        let scope = self.scope_for(self.tabs[idx].collection_id.as_deref());
                        let mut actions = Vec::new();
                        request::url_bar(ui, &mut self.tabs[idx], &scope, &mut actions);
                        ui.add_space(6.0);
                        let tab = &mut self.tabs[idx];
                        let avail = ui.available_height();
                        egui::TopBottomPanel::bottom("response-panel")
                            .resizable(true)
                            .default_height((avail * 0.58).max(320.0))
                            .height_range(80.0..=(avail - 80.0).max(100.0))
                            .frame(egui::Frame::new().fill(p.surface).inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                            .show_inside(ui, |ui| {
                                // Fill the panel so egui remembers the dragged height, not the content height.
                                ui.set_min_height(ui.max_rect().height());
                                if tab.request.kind == RequestKind::WebSocket {
                                    request::ws_log(ui, tab);
                                } else {
                                    response::show(ui, tab);
                                }
                            });
                        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.surface)).show_inside(ui, |ui| {
                            request::sections(ui, tab, &scope, &mut actions);
                        });
                        self.actions.extend(actions);
                    });
            });
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if self.toasts.is_empty() {
            return;
        }
        let p = theme::palette(self.ws.settings.dark_mode);
        egui::Area::new("toasts".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -32.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &self.toasts {
                    egui::Frame::new()
                        .fill(p.surface_hi)
                        .stroke(egui::Stroke::new(1.0_f32, if t.error { p.err } else { p.border }))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.label(RichText::new(&t.text).color(if t.error { p.err } else { p.text }));
                        });
                    ui.add_space(6.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn autosave(&mut self) {
        if self.dirty && self.last_change.elapsed() > Duration::from_millis(400) {
            self.dirty = false;
            let _ = self.saver.send(SaveMsg::Workspace(Box::new(self.ws.clone())));
        }
        if self.last_ui_save.elapsed() > Duration::from_secs(3) {
            self.last_ui_save = Instant::now();
            let _ = self.saver.send(SaveMsg::Ui(self.ui_state()));
        }
    }
}

impl eframe::App for SwiftApp {
    /// Window size is still remembered; transient widget state (panel splits, scroll) starts fresh.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let want = (self.ws.settings.dark_mode, self.ws.settings.ui_scale);
        if self.applied_theme != Some(want) {
            theme::apply(ctx, want.0);
            ctx.set_zoom_factor(want.1.clamp(0.7, 2.0));
            self.applied_theme = Some(want);
        }
        self.handle_events();
        self.shortcuts(ctx);

        self.top_bar(ctx);
        self.status_bar(ctx);
        self.sidebar(ctx);
        self.central(ctx);

        self.dialogs(ctx);
        self.command_palette(ctx);
        self.toasts(ctx);

        self.process_actions();
        self.autosave();
        // Keep polling the event channel while work is in flight.
        if self.tabs.iter().any(|t| t.pending.is_some() || t.ws.connecting) || self.runner.running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl Drop for SwiftApp {
    fn drop(&mut self) {
        if let Err(e) = self.storage.save(&self.ws) {
            eprintln!("tessera: final save failed: {e}");
        }
        let _ = self.storage.save_ui(&self.ui_state());
    }
}

pub fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect()
}
