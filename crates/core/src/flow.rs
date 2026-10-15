//! Flows: requests wired together on a canvas, passing values (tokens, ids…) from one response to the next.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{HttpEngine, HttpResponse};
use crate::model::{new_id, vars_to_map, ClientCert, Collection, Request, RequestKind, Settings};
use crate::script::{ScriptInfo, ScriptVars, TestResult};
use crate::send::{send, SendContext};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapSource {
    /// Path into the JSON body, e.g. `access_token` or `data.items[0].id`.
    #[default]
    Body,
    Header,
    Status,
}

impl MapSource {
    pub const ALL: [MapSource; 3] = [MapSource::Body, MapSource::Header, MapSource::Status];
    pub fn label(self) -> &'static str {
        match self {
            MapSource::Body => "Body (JSON)",
            MapSource::Header => "Header",
            MapSource::Status => "Status code",
        }
    }
}

/// "Take `path` from the source response and put it in `{{variable}}`".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mapping {
    pub source: MapSource,
    pub path: String,
    pub variable: String,
    /// Also write the value into the active environment, so later manual sends can use it.
    pub save_to_env: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    pub collection_id: String,
    pub request_id: String,
    pub pos: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub mappings: Vec<Mapping>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
    #[serde(default)]
    pub description: String,
}

impl Flow {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: new_id(), name: name.into(), nodes: Vec::new(), edges: Vec::new(), description: String::new() }
    }

    pub fn node(&self, id: &str) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// True when `to` can already reach `from` — adding `from → to` would create a loop.
    pub fn would_cycle(&self, from: &str, to: &str) -> bool {
        if from == to {
            return true;
        }
        let mut stack = vec![to.to_string()];
        let mut seen = HashSet::new();
        while let Some(n) = stack.pop() {
            if n == from {
                return true;
            }
            if seen.insert(n.clone()) {
                stack.extend(self.edges.iter().filter(|e| e.from == n).map(|e| e.to.clone()));
            }
        }
        false
    }

    /// Adds an edge unless it already exists or would create a cycle. Returns the edge id.
    pub fn connect(&mut self, from: &str, to: &str) -> Option<String> {
        if self.edges.iter().any(|e| e.from == from && e.to == to) || self.would_cycle(from, to) {
            return None;
        }
        let id = new_id();
        self.edges.push(FlowEdge { id: id.clone(), from: from.into(), to: to.into(), mappings: Vec::new() });
        Some(id)
    }

    pub fn remove_node(&mut self, id: &str) {
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
    }

    /// Topological order; ties broken left-to-right, top-to-bottom so runs follow the visual layout.
    pub fn order(&self) -> Result<Vec<String>, String> {
        let mut indeg: HashMap<&str, usize> = self.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
        for e in &self.edges {
            if let Some(d) = indeg.get_mut(e.to.as_str()) {
                *d += 1;
            }
        }
        let key = |id: &str| self.node(id).map(|n| (n.pos[0], n.pos[1])).unwrap_or((0.0, 0.0));
        let mut ready: Vec<&str> = indeg.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
        let mut out = Vec::new();
        while !ready.is_empty() {
            ready.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
            let id = ready.remove(0);
            out.push(id.to_string());
            for e in self.edges.iter().filter(|e| e.from == id) {
                if let Some(d) = indeg.get_mut(e.to.as_str()) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push(e.to.as_str());
                    }
                }
            }
        }
        if out.len() != self.nodes.len() {
            return Err("The flow contains a loop".into());
        }
        Ok(out)
    }
}

/// Look up a value with a simple path: `token`, `$.data.items[0].id`, `items[2]`, `["odd key"]`.
pub fn extract_json<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let p = path.trim();
    let p = p.strip_prefix('$').unwrap_or(p);
    let mut cur = v;
    let chars: Vec<char> = p.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => i += 1,
            '[' => {
                let end = chars[i..].iter().position(|&c| c == ']')? + i;
                let inner: String = chars[i + 1..end].iter().collect();
                let inner = inner.trim();
                cur = if let Ok(idx) = inner.parse::<usize>() {
                    cur.get(idx)?
                } else {
                    cur.get(inner.trim_matches(|c| c == '"' || c == '\''))?
                };
                i = end + 1;
            }
            _ => {
                let end = chars[i..].iter().position(|&c| c == '.' || c == '[').map(|e| e + i).unwrap_or(chars.len());
                let key: String = chars[i..end].iter().collect();
                cur = cur.get(key.as_str())?;
                i = end;
            }
        }
    }
    Some(cur)
}

/// Resolve one mapping against a response.
pub fn extract(m: &Mapping, resp: &HttpResponse, json: Option<&Value>) -> Result<String, String> {
    match m.source {
        MapSource::Status => Ok(resp.status.to_string()),
        MapSource::Header => resp.header(m.path.trim()).map(str::to_string).ok_or_else(|| format!("header '{}' not in response", m.path)),
        MapSource::Body => {
            let json = json.ok_or("response body is not JSON")?;
            match extract_json(json, &m.path) {
                Some(Value::String(s)) => Ok(s.clone()),
                Some(Value::Null) | None => Err(format!("'{}' not found in response", m.path)),
                Some(other) => Ok(other.to_string()),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    Running,
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug)]
pub struct NodeResult {
    pub status: NodeStatus,
    pub http_status: Option<u16>,
    pub time_ms: f64,
    pub error: Option<String>,
    pub tests: Vec<TestResult>,
    /// Variables this node's outgoing wires produced.
    pub extracted: Vec<(String, String)>,
    pub response: Option<HttpResponse>,
    pub json: Option<Value>,
}

impl NodeResult {
    pub fn running() -> Self {
        Self { status: NodeStatus::Running, error: None, ..Self::skipped("") }
    }

    fn skipped(reason: impl Into<String>) -> Self {
        Self {
            status: NodeStatus::Skipped,
            http_status: None,
            time_ms: 0.0,
            error: Some(reason.into()),
            tests: Vec::new(),
            extracted: Vec::new(),
            response: None,
            json: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FlowEvent {
    NodeStarted(String),
    NodeFinished(String, Box<NodeResult>),
    /// Final variable state: env/globals plus each collection's variables, and the flow's own values.
    Finished { vars: ScriptVars, collections: HashMap<String, BTreeMap<String, String>>, flow_vars: BTreeMap<String, String>, error: Option<String>, cancelled: bool },
}

pub struct FlowRunConfig {
    pub flow: Flow,
    pub collections: Vec<Collection>,
    /// Env + globals (collection vars are taken from each node's collection).
    pub vars: ScriptVars,
    pub env_name: String,
    pub settings: Settings,
    pub certs: Vec<ClientCert>,
}

fn find_request<'a>(cols: &'a [Collection], n: &FlowNode) -> Option<(&'a Collection, &'a Request)> {
    let c = cols.iter().find(|c| c.id == n.collection_id)?;
    Some((c, c.find_request(&n.request_id)?))
}

pub async fn run_flow(engine: Arc<HttpEngine>, cfg: FlowRunConfig, cancel: Arc<AtomicBool>, emit: impl Fn(FlowEvent)) {
    let mut collection_vars: HashMap<String, BTreeMap<String, String>> =
        cfg.collections.iter().map(|c| (c.id.clone(), vars_to_map(&c.variables))).collect();
    let mut vars = cfg.vars.clone();
    let mut flow_vars: BTreeMap<String, String> = BTreeMap::new();
    let finish = |vars: ScriptVars, cols, flow_vars, error, cancelled| {
        emit(FlowEvent::Finished { vars, collections: cols, flow_vars, error, cancelled })
    };
    let order = match cfg.flow.order() {
        Ok(o) => o,
        Err(e) => return finish(vars, collection_vars, flow_vars, Some(e), false),
    };
    let mut ok: HashSet<String> = HashSet::new();

    for node_id in order {
        if cancel.load(Ordering::Relaxed) {
            return finish(vars, collection_vars, flow_vars, None, true);
        }
        let node = cfg.flow.node(&node_id).cloned().expect("node in order");
        let upstream: Vec<&FlowEdge> = cfg.flow.edges.iter().filter(|e| e.to == node_id).collect();
        if let Some(e) = upstream.iter().find(|e| !ok.contains(&e.from)) {
            let name = cfg.flow.node(&e.from).and_then(|n| find_request(&cfg.collections, n)).map(|(_, r)| r.name.clone()).unwrap_or_default();
            emit(FlowEvent::NodeFinished(node_id.clone(), Box::new(NodeResult::skipped(format!("Skipped: '{name}' did not succeed")))));
            continue;
        }
        let Some((collection, request)) = find_request(&cfg.collections, &node) else {
            emit(FlowEvent::NodeFinished(node_id.clone(), Box::new(NodeResult { status: NodeStatus::Failed, ..NodeResult::skipped("Request no longer exists") })));
            continue;
        };
        if request.kind == RequestKind::WebSocket {
            emit(FlowEvent::NodeFinished(node_id.clone(), Box::new(NodeResult { status: NodeStatus::Failed, ..NodeResult::skipped("WebSocket requests can't run in a flow") })));
            continue;
        }
        emit(FlowEvent::NodeStarted(node_id.clone()));

        let mut node_vars = vars.clone();
        node_vars.collection = collection_vars.get(&collection.id).cloned().unwrap_or_default();
        node_vars.locals.clear();
        let ctx = SendContext {
            request: request.clone(),
            auth: collection.effective_auth(request),
            parent_scripts: collection.parent_scripts(&request.id),
            vars: node_vars,
            // Values carried along wires; they win over environment/collection variables.
            data: flow_vars.clone(),
            env_name: cfg.env_name.clone(),
            settings: cfg.settings.clone(),
            certs: cfg.certs.clone(),
            info: ScriptInfo { request_name: request.name.clone(), request_id: request.id.clone(), iteration_count: 1, ..Default::default() },
        };
        let out = send(&engine, ctx).await;
        vars.env = out.vars.env.clone();
        vars.globals = out.vars.globals.clone();
        collection_vars.insert(collection.id.clone(), out.vars.collection.clone());

        let mut result = NodeResult {
            status: NodeStatus::Passed,
            http_status: None,
            time_ms: 0.0,
            error: None,
            tests: out.tests.clone(),
            extracted: Vec::new(),
            response: None,
            json: None,
        };
        match out.response {
            Err(e) => {
                result.status = NodeStatus::Failed;
                result.error = Some(e);
            }
            Ok(resp) => {
                result.http_status = Some(resp.status);
                result.time_ms = resp.time_ms;
                result.json = serde_json::from_slice(&resp.body).ok();
                if resp.status >= 400 {
                    result.status = NodeStatus::Failed;
                    result.error = Some(format!("HTTP {} {}", resp.status, resp.status_text));
                } else if result.tests.iter().any(|t| !t.passed) {
                    result.status = NodeStatus::Failed;
                    result.error = Some("Tests failed".into());
                }
                if result.status == NodeStatus::Passed {
                    'edges: for e in cfg.flow.edges.iter().filter(|e| e.from == node_id) {
                        for m in e.mappings.iter().filter(|m| !m.variable.trim().is_empty()) {
                            match extract(m, &resp, result.json.as_ref()) {
                                Ok(v) => {
                                    let name = m.variable.trim().trim_matches(|c| c == '{' || c == '}').to_string();
                                    flow_vars.insert(name.clone(), v.clone());
                                    if m.save_to_env {
                                        vars.env.insert(name.clone(), v.clone());
                                    }
                                    result.extracted.push((name, v));
                                }
                                Err(err) => {
                                    result.status = NodeStatus::Failed;
                                    result.error = Some(format!("Could not pass {{{{{}}}}}: {err}", m.variable.trim()));
                                    break 'edges;
                                }
                            }
                        }
                    }
                }
                result.response = Some(resp);
            }
        }
        if result.status == NodeStatus::Passed {
            ok.insert(node_id.clone());
        }
        emit(FlowEvent::NodeFinished(node_id, Box::new(result)));
    }
    finish(vars, collection_vars, flow_vars, None, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, x: f32) -> FlowNode {
        FlowNode { id: id.into(), collection_id: String::new(), request_id: String::new(), pos: [x, 0.0] }
    }

    #[test]
    fn json_paths() {
        let v: Value = serde_json::json!({"access_token": "t", "data": {"items": [{"id": 7}, {"id": 8}]}, "odd key": 1});
        assert_eq!(extract_json(&v, "access_token"), Some(&Value::from("t")));
        assert_eq!(extract_json(&v, "$.data.items[1].id"), Some(&Value::from(8)));
        assert_eq!(extract_json(&v, "data.items[0]"), Some(&serde_json::json!({"id": 7})));
        assert_eq!(extract_json(&v, "[\"odd key\"]"), Some(&Value::from(1)));
        assert_eq!(extract_json(&v, "data.missing"), None);
    }

    #[test]
    fn ordering_and_cycles() {
        let mut f = Flow::new("f");
        f.nodes = vec![node("c", 300.0), node("a", 0.0), node("b", 150.0), node("d", 50.0)];
        assert!(f.connect("a", "b").is_some());
        assert!(f.connect("b", "c").is_some());
        assert!(f.connect("a", "b").is_none(), "duplicate");
        assert!(f.connect("c", "a").is_none(), "cycle");
        assert!(f.connect("a", "a").is_none(), "self loop");
        assert_eq!(f.order().unwrap(), vec!["a", "d", "b", "c"]);
        f.remove_node("b");
        assert!(f.edges.is_empty());
    }
}
