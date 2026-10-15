//! Collection runner: execute every request of a collection/folder, optionally with a data file.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::http::HttpEngine;
use crate::model::{ClientCert, Collection, Item, Request, RequestKind, Settings};
use crate::script::{ScriptInfo, ScriptVars, TestResult};
use crate::send::{send, SendContext};

#[derive(Clone, Debug)]
pub struct RunConfig {
    pub collection: Collection,
    /// Run only this folder (and its descendants).
    pub folder_id: Option<String>,
    pub iterations: usize,
    pub delay_ms: u64,
    pub data: Vec<BTreeMap<String, String>>,
    pub vars: ScriptVars,
    pub env_name: String,
    pub settings: Settings,
    pub certs: Vec<ClientCert>,
    pub stop_on_error: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RunResult {
    pub iteration: usize,
    pub request_id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub time_ms: f64,
    pub size: usize,
    pub tests: Vec<TestResult>,
}

#[derive(Clone, Debug)]
pub enum RunEvent {
    Started { total: usize },
    Result(RunResult),
    Finished { vars: ScriptVars, cancelled: bool },
}

fn requests_of(items: &[Item], out: &mut Vec<Request>) {
    for i in items {
        match i {
            Item::Request(r) if r.kind != RequestKind::WebSocket => out.push(r.clone()),
            Item::Request(_) => {}
            Item::Folder(f) => requests_of(&f.items, out),
        }
    }
}

pub fn runnable_requests(collection: &Collection, folder_id: Option<&str>) -> Vec<Request> {
    let mut out = Vec::new();
    match folder_id {
        Some(fid) => {
            let mut c = collection.clone();
            if let Some(f) = c.find_folder_mut(fid) {
                requests_of(&f.items, &mut out);
            }
        }
        None => requests_of(&collection.items, &mut out),
    }
    out
}

pub async fn run(engine: Arc<HttpEngine>, cfg: RunConfig, cancel: Arc<AtomicBool>, emit: impl Fn(RunEvent)) {
    let requests = runnable_requests(&cfg.collection, cfg.folder_id.as_deref());
    let iterations = if cfg.iterations == 0 { cfg.data.len().max(1) } else { cfg.iterations };
    emit(RunEvent::Started { total: requests.len() * iterations });
    let mut vars = cfg.vars.clone();
    let mut cancelled = false;

    'outer: for iteration in 0..iterations {
        let data = if cfg.data.is_empty() { BTreeMap::new() } else { cfg.data[iteration % cfg.data.len()].clone() };
        let mut idx = 0;
        let mut steps = 0;
        while idx < requests.len() {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break 'outer;
            }
            steps += 1;
            if steps > requests.len() * 50 {
                break; // guard against setNextRequest loops
            }
            let req = &requests[idx];
            vars.locals.clear();
            let ctx = SendContext {
                request: req.clone(),
                auth: cfg.collection.effective_auth(req),
                parent_scripts: cfg.collection.parent_scripts(&req.id),
                vars: vars.clone(),
                data: data.clone(),
                env_name: cfg.env_name.clone(),
                settings: cfg.settings.clone(),
                certs: cfg.certs.clone(),
                info: ScriptInfo {
                    event_name: String::new(),
                    request_name: req.name.clone(),
                    request_id: req.id.clone(),
                    iteration,
                    iteration_count: iterations,
                },
            };
            let out = send(&engine, ctx).await;
            vars = out.vars.clone();
            let mut result = RunResult {
                iteration,
                request_id: req.id.clone(),
                name: req.name.clone(),
                method: out.prepared.as_ref().map(|p| p.method.clone()).unwrap_or_else(|| req.method.clone()),
                url: out.prepared.as_ref().map(|p| p.url.clone()).unwrap_or_else(|| req.url.clone()),
                tests: out.tests.clone(),
                ..Default::default()
            };
            match &out.response {
                Ok(r) => {
                    result.status = Some(r.status);
                    result.time_ms = r.time_ms;
                    result.size = r.size();
                }
                Err(e) => result.error = Some(e.clone()),
            }
            let failed = result.error.is_some() && !out.skipped || result.tests.iter().any(|t| !t.passed);
            emit(RunEvent::Result(result));
            if failed && cfg.stop_on_error {
                break 'outer;
            }
            idx = match out.next_request {
                None => idx + 1,
                Some(None) => break,
                Some(Some(name)) => match requests.iter().position(|r| r.name == name) {
                    Some(p) => p,
                    None => idx + 1,
                },
            };
            if cfg.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(cfg.delay_ms)).await;
            }
        }
    }
    emit(RunEvent::Finished { vars, cancelled });
}

/// Parse a CSV (first row = headers) or JSON array-of-objects data file.
pub fn parse_data_file(text: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('[') {
        let v: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| format!("JSON data file: {e}"))?;
        let arr = v.as_array().ok_or("JSON data file must be an array of objects")?;
        return Ok(arr
            .iter()
            .filter_map(|row| row.as_object())
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
                    .collect()
            })
            .collect());
    }
    let rows = parse_csv(trimmed);
    let mut it = rows.into_iter();
    let header = it.next().ok_or("Data file is empty")?;
    Ok(it
        .filter(|r| !(r.len() == 1 && r[0].is_empty()))
        .map(|r| header.iter().cloned().zip(r.into_iter().chain(std::iter::repeat(String::new()))).collect())
        .collect())
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, in_quotes) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => in_quotes = false,
            ('"', false) if field.is_empty() => in_quotes = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_with_quotes() {
        let d = parse_data_file("id,name\n1,\"Doe, \"\"J\"\"\"\n2,Ann\n").unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["name"], "Doe, \"J\"");
        assert_eq!(d[1]["id"], "2");
    }

    #[test]
    fn json_data() {
        let d = parse_data_file(r#"[{"a":1,"b":"x"}]"#).unwrap();
        assert_eq!(d[0]["a"], "1");
        assert_eq!(d[0]["b"], "x");
    }
}
