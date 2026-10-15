//! The full send pipeline: pre-request scripts → resolve → HTTP → test scripts.

use std::collections::BTreeMap;

use crate::http::{prepare, HttpEngine, HttpResponse, PreparedRequest};
use crate::model::{Auth, ClientCert, KeyValue, Request, Scripts, Settings};
use crate::script::{
    run_scripts, ConsoleLine, Pair, ScriptInfo, ScriptRequest, ScriptResponse, ScriptVars, TestResult,
};
use crate::vars::VarScope;

#[derive(Clone, Debug, Default)]
pub struct SendContext {
    pub request: Request,
    /// Auth after resolving inheritance.
    pub auth: Auth,
    /// Collection and folder scripts, outermost first.
    pub parent_scripts: Vec<Scripts>,
    pub vars: ScriptVars,
    pub data: BTreeMap<String, String>,
    pub env_name: String,
    pub settings: Settings,
    pub certs: Vec<ClientCert>,
    pub info: ScriptInfo,
}

impl SendContext {
    pub fn scope(&self) -> VarScope {
        VarScope {
            local: self.vars.locals.clone(),
            data: self.data.clone(),
            environment: self.vars.env.clone(),
            collection: self.vars.collection.clone(),
            globals: self.vars.globals.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SendOutcome {
    pub response: Result<HttpResponse, String>,
    pub prepared: Option<PreparedRequest>,
    pub tests: Vec<TestResult>,
    pub console: Vec<ConsoleLine>,
    pub vars: ScriptVars,
    pub next_request: Option<Option<String>>,
    pub skipped: bool,
}

fn has_code(scripts: &[String]) -> bool {
    scripts.iter().any(|s| !s.trim().is_empty())
}

pub async fn send(engine: &HttpEngine, mut ctx: SendContext) -> SendOutcome {
    let mut console = Vec::new();
    let mut tests = Vec::new();
    let mut next_request = None;

    // ---- pre-request scripts ----
    let mut pre: Vec<String> = ctx.parent_scripts.iter().map(|s| s.pre_request.clone()).collect();
    pre.push(ctx.request.scripts.pre_request.clone());
    if has_code(&pre) {
        let sreq = ScriptRequest {
            method: ctx.request.method.clone(),
            url: ctx.request.url.clone(),
            headers: ctx
                .request
                .headers
                .iter()
                .filter(|h| h.enabled && !h.key.is_empty())
                .map(|h| Pair { key: h.key.clone(), value: h.value.clone() })
                .collect(),
            body: ctx.request.body.raw.clone(),
        };
        let mut info = ctx.info.clone();
        info.event_name = "prerequest".into();
        let (vars, data, env_name) = (ctx.vars.clone(), ctx.data.clone(), ctx.env_name.clone());
        let joined = tokio::task::spawn_blocking(move || {
            let mut vars = vars;
            let mut sreq = sreq;
            let refs: Vec<&str> = pre.iter().map(String::as_str).collect();
            let out = run_scripts(&refs, &mut vars, &data, &env_name, &mut sreq, None, &info);
            (vars, sreq, out)
        })
        .await;
        match joined {
            Ok((vars, sreq, out)) => {
                ctx.vars = vars;
                ctx.request.method = sreq.method;
                ctx.request.url = sreq.url;
                let disabled: Vec<KeyValue> = ctx.request.headers.iter().filter(|h| !h.enabled).cloned().collect();
                ctx.request.headers = sreq.headers.into_iter().map(|p| KeyValue::new(p.key, p.value)).collect();
                ctx.request.headers.extend(disabled);
                console.extend(out.console);
                tests.extend(out.tests);
                next_request = out.next_request;
                if out.skip_request {
                    return SendOutcome {
                        response: Err("Request skipped by pre-request script".into()),
                        prepared: None,
                        tests,
                        console,
                        vars: ctx.vars,
                        next_request,
                        skipped: true,
                    };
                }
            }
            Err(e) => console.push(ConsoleLine { level: "error".into(), text: format!("Script thread: {e}") }),
        }
    }

    // ---- HTTP ----
    let scope = ctx.scope();
    let prepared = prepare(&ctx.request, &ctx.auth, &scope, &ctx.settings);
    let response = engine.execute(&prepared, &ctx.settings, &ctx.certs).await;

    // ---- test scripts ----
    let mut post: Vec<String> = ctx.parent_scripts.iter().map(|s| s.test.clone()).collect();
    post.push(ctx.request.scripts.test.clone());
    if let (true, Ok(resp)) = (has_code(&post), &response) {
        let sresp = ScriptResponse {
            code: resp.status,
            status: resp.status_text.clone(),
            headers: resp.headers.iter().map(|(k, v)| Pair { key: k.clone(), value: v.clone() }).collect(),
            body: resp.text(),
            response_time: resp.time_ms.round(),
            response_size: resp.size(),
            cookies: resp.cookies.iter().map(|c| Pair { key: c.name.clone(), value: c.value.clone() }).collect(),
        };
        let sreq = ScriptRequest {
            method: prepared.method.clone(),
            url: prepared.url.clone(),
            headers: prepared.headers.iter().map(|(k, v)| Pair { key: k.clone(), value: v.clone() }).collect(),
            body: match &prepared.body {
                crate::http::PreparedBody::Text(t) => t.clone(),
                _ => String::new(),
            },
        };
        let mut info = ctx.info.clone();
        info.event_name = "test".into();
        let (vars, data, env_name) = (ctx.vars.clone(), ctx.data.clone(), ctx.env_name.clone());
        let joined = tokio::task::spawn_blocking(move || {
            let mut vars = vars;
            let mut sreq = sreq;
            let refs: Vec<&str> = post.iter().map(String::as_str).collect();
            let out = run_scripts(&refs, &mut vars, &data, &env_name, &mut sreq, Some(&sresp), &info);
            (vars, out)
        })
        .await;
        match joined {
            Ok((vars, out)) => {
                ctx.vars = vars;
                console.extend(out.console);
                tests.extend(out.tests);
                if out.next_request.is_some() {
                    next_request = out.next_request;
                }
            }
            Err(e) => console.push(ConsoleLine { level: "error".into(), text: format!("Script thread: {e}") }),
        }
    }

    SendOutcome { response, prepared: Some(prepared), tests, console, vars: ctx.vars, next_request, skipped: false }
}
