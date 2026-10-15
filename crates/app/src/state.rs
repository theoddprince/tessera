//! UI-side state: tabs, dialogs, runner.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tessera_core::graphql::Schema;
use tessera_core::http::{HttpResponse, PreparedRequest};
use tessera_core::model::{Request, RequestKind};
use tessera_core::runner::RunResult;
use tessera_core::script::{ConsoleLine, TestResult};
use tessera_core::ws::WsCommand;

use crate::view::BodyView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReqSection {
    #[default]
    Params,
    Auth,
    Headers,
    Body,
    Scripts,
    Settings,
    Query,
    Variables,
    Messages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RespSection {
    #[default]
    Body,
    Headers,
    Cookies,
    Tests,
    Console,
    Request,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BodyMode {
    #[default]
    Pretty,
    Raw,
    Tree,
    Preview,
}

pub struct ResponseState {
    pub result: Result<HttpResponse, String>,
    pub prepared: Option<PreparedRequest>,
    pub tests: Vec<TestResult>,
    pub console: Vec<ConsoleLine>,
    pub view: BodyView,
    pub search: String,
    pub search_hit: usize,
    pub scroll_to: Option<usize>,
}

pub struct Pending {
    pub handle: tokio::task::AbortHandle,
    pub started: Instant,
}

#[derive(Clone, Debug)]
pub struct WsLine {
    pub incoming: bool,
    pub system: bool,
    pub text: String,
    pub at: String,
}

#[derive(Default)]
pub struct WsState {
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<WsCommand>>,
    pub handle: Option<tokio::task::AbortHandle>,
    pub connected: bool,
    pub connecting: bool,
    pub log: Vec<WsLine>,
    pub compose: String,
    pub filter: String,
}

pub struct Tab {
    pub id: u64,
    /// Set when this tab shows a flow canvas instead of a request.
    pub flow_id: Option<String>,
    /// Collection the request is saved in.
    pub collection_id: Option<String>,
    pub request: Request,
    /// Last saved copy (for the unsaved-changes dot).
    pub saved: Option<Request>,
    pub req_section: ReqSection,
    pub resp_section: RespSection,
    pub body_mode: BodyMode,
    pub response: Option<ResponseState>,
    pub pending: Option<Pending>,
    pub ws: WsState,
    pub schema: Option<Result<Schema, String>>,
    pub schema_loading: bool,
    pub schema_filter: String,
    pub oauth_busy: bool,
    pub oauth_msg: Option<Result<String, String>>,
}

impl Tab {
    pub fn new(id: u64, request: Request, collection_id: Option<String>, saved: bool) -> Self {
        let req_section = match request.kind {
            RequestKind::Http => ReqSection::Params,
            RequestKind::GraphQL => ReqSection::Query,
            RequestKind::WebSocket => ReqSection::Messages,
        };
        Self {
            id,
            flow_id: None,
            collection_id,
            saved: saved.then(|| request.clone()),
            request,
            req_section,
            resp_section: RespSection::Body,
            body_mode: BodyMode::Pretty,
            response: None,
            pending: None,
            ws: WsState::default(),
            schema: None,
            schema_loading: false,
            schema_filter: String::new(),
            oauth_busy: false,
            oauth_msg: None,
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.saved {
            Some(s) => s != &self.request,
            None => !self.request.url.trim().is_empty(),
        }
    }

    pub fn title(&self) -> String {
        if self.request.name.trim().is_empty() {
            if self.request.url.is_empty() {
                "Untitled".into()
            } else {
                self.request.url.clone()
            }
        } else {
            self.request.name.clone()
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedTab {
    pub flow_id: Option<String>,
    pub collection_id: Option<String>,
    pub request: Option<Request>,
    pub saved: Option<Request>,
    pub req_section: ReqSection,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub tabs: Vec<PersistedTab>,
    pub active: usize,
    pub expanded: Vec<String>,
}

pub struct RunnerState {
    pub open: bool,
    pub collection_id: Option<String>,
    pub folder_id: Option<String>,
    pub iterations: usize,
    pub delay_ms: u64,
    pub data_path: String,
    pub data: Vec<BTreeMap<String, String>>,
    pub data_error: Option<String>,
    pub stop_on_error: bool,
    pub persist_vars: bool,
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
    pub total: usize,
    pub results: Vec<RunResult>,
    pub started: Option<Instant>,
    pub elapsed_ms: f64,
    pub selected: Option<usize>,
    pub only_failed: bool,
}

impl Default for RunnerState {
    fn default() -> Self {
        Self {
            open: false,
            collection_id: None,
            folder_id: None,
            iterations: 1,
            delay_ms: 0,
            data_path: String::new(),
            data: Vec::new(),
            data_error: None,
            stop_on_error: false,
            persist_vars: true,
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            total: 0,
            results: Vec::new(),
            started: None,
            elapsed_ms: 0.0,
            selected: None,
            only_failed: false,
        }
    }
}

pub struct SaveAs {
    pub tab_id: u64,
    pub name: String,
    pub collection_id: Option<String>,
    pub folder_id: Option<String>,
    pub new_collection: String,
}

pub struct ItemSettings {
    pub collection_id: String,
    pub folder_id: Option<String>,
    pub section: ItemSection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ItemSection {
    Overview,
    Auth,
    Scripts,
    Variables,
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub focus_pending: bool,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}
