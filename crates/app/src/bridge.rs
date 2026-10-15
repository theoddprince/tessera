//! Background tokio runtime and the event channel back to the UI thread.

use std::future::Future;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use tessera_core::graphql::Schema;
use tessera_core::http::HttpEngine;
use tessera_core::oauth::OAuthToken;
use tessera_core::runner::RunEvent;
use tessera_core::send::SendOutcome;
use tessera_core::ws::WsEvent;

use crate::view::BodyView;

/// Where an OAuth token should be written once it arrives.
#[derive(Clone, Debug, PartialEq)]
pub enum AuthOwner {
    Tab(u64),
    Collection(String),
    Folder { collection: String, folder: String },
}

pub enum AppEvent {
    Http { tab_id: u64, outcome: Box<SendOutcome>, view: Box<BodyView> },
    OAuth { owner: AuthOwner, result: Result<OAuthToken, String> },
    Ws { tab_id: u64, event: WsEvent },
    Schema { tab_id: u64, result: Result<Schema, String> },
    Run(RunEvent),
    Flow { flow_id: String, event: tessera_core::flow::FlowEvent },
}

#[derive(Clone)]
pub struct Emitter {
    tx: Sender<AppEvent>,
    ctx: egui::Context,
}

impl Emitter {
    pub fn emit(&self, e: AppEvent) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }
}

pub struct Bridge {
    rt: tokio::runtime::Runtime,
    rx: Receiver<AppEvent>,
    emitter: Emitter,
    pub engine: Arc<HttpEngine>,
}

impl Bridge {
    pub fn new(ctx: egui::Context) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("tessera-io")
            .build()
            .expect("tokio runtime");
        let (tx, rx) = channel();
        Self { rt, rx, emitter: Emitter { tx, ctx }, engine: Arc::new(HttpEngine::default()) }
    }

    pub fn emitter(&self) -> Emitter {
        self.emitter.clone()
    }

    pub fn spawn<F>(&self, fut: F) -> tokio::task::AbortHandle
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.rt.spawn(fut).abort_handle()
    }

    pub fn drain(&self) -> Vec<AppEvent> {
        self.rx.try_iter().collect()
    }
}
