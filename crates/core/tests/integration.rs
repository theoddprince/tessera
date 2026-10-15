//! End-to-end tests against a local HTTP/WebSocket server (no internet needed).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tessera_core::http::HttpEngine;
use tessera_core::model::*;
use tessera_core::runner::{run, RunConfig, RunEvent};
use tessera_core::script::ScriptVars;
use tessera_core::send::{send, SendContext};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Minimal HTTP/1.1 server: echoes the request as JSON, plus a few special paths.
async fn http_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (head_end, content_len) = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while buf.len() < head_end + content_len {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end - 4]).to_string();
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or("").split_whitespace();
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                let headers: serde_json::Map<String, serde_json::Value> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), serde_json::Value::String(v.trim().to_string())))
                    .collect();
                let (status, extra, payload) = if path.starts_with("/status/404") {
                    ("404 Not Found", String::new(), r#"{"error":"nope"}"#.to_string())
                } else if path.starts_with("/redirect") {
                    ("302 Found", "Location: /echo?from=redirect\r\n".to_string(), String::new())
                } else if path.starts_with("/login") {
                    ("200 OK", "Set-Cookie: sid=abc123; Path=/; HttpOnly\r\n".to_string(), r#"{"token":"tok-42"}"#.to_string())
                } else {
                    let v = serde_json::json!({ "method": method, "path": path, "headers": headers, "body": body });
                    ("200 OK", String::new(), v.to_string())
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{payload}",
                    payload.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

async fn ws_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    if msg.is_text() {
                        let reply = format!("echo: {}", msg.to_text().unwrap());
                        ws.send(tokio_tungstenite::tungstenite::Message::Text(reply)).await.unwrap();
                    }
                }
            });
        }
    });
    format!("ws://{addr}")
}

fn ctx_for(req: Request, base: &str) -> SendContext {
    let mut vars = ScriptVars::default();
    vars.env.insert("base".into(), base.into());
    SendContext { auth: if req.auth.kind == AuthKind::Inherit { Auth::none() } else { req.auth.clone() }, request: req, vars, ..Default::default() }
}

#[tokio::test]
async fn full_pipeline_with_scripts_and_vars() {
    let base = http_server().await;
    let engine = HttpEngine::default();
    let mut req = Request { method: "POST".into(), url: "{{base}}/echo?q={{$timestamp}}".into(), ..Default::default() };
    req.body.mode = BodyMode::Raw;
    req.body.raw = r#"{"user":"{{user}}"}"#.into();
    req.scripts.pre_request = r#"
        pm.variables.set("user", "ann");
        pm.request.headers.add({key: "X-Trace", value: "t-1"});
    "#
    .into();
    req.scripts.test = r#"
        pm.test("status", () => pm.response.to.have.status(200));
        const j = pm.response.json();
        pm.test("header from pre-request", () => pm.expect(j.headers["x-trace"]).to.equal("t-1"));
        pm.test("body resolved", () => pm.expect(JSON.parse(j.body)).to.eql({user: "ann"}));
        pm.test("content-type auto", () => pm.expect(j.headers["content-type"]).to.include("json"));
        pm.environment.set("lastMethod", j.method);
    "#
    .into();
    let out = send(&engine, ctx_for(req, &base)).await;
    let resp = out.response.expect("response");
    assert_eq!(resp.status, 200);
    assert_eq!(out.tests.len(), 4, "{:?}", out.tests);
    assert!(out.tests.iter().all(|t| t.passed), "{:?} {:?}", out.tests, out.console);
    assert_eq!(out.vars.env.get("lastMethod").map(String::as_str), Some("POST"));
}

#[tokio::test]
async fn forms_auth_cookies_and_redirects() {
    let base = http_server().await;
    let engine = HttpEngine::default();

    let mut form = Request { method: "POST".into(), url: "{{base}}/form".into(), ..Default::default() };
    form.body.mode = BodyMode::FormUrlEncoded;
    form.body.form = vec![KeyValue::new("a", "1 2"), KeyValue { enabled: false, ..KeyValue::new("skip", "x") }];
    form.auth = Auth { kind: AuthKind::Basic, basic_user: "u".into(), basic_pass: "p".into(), ..Default::default() };
    let r = send(&engine, ctx_for(form, &base)).await.response.unwrap();
    let j: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(j["body"], "a=1+2");
    assert_eq!(j["headers"]["authorization"], "Basic dTpw");
    assert!(j["headers"]["content-type"].as_str().unwrap().contains("x-www-form-urlencoded"));

    let mut mp = Request { method: "POST".into(), url: "{{base}}/upload".into(), ..Default::default() };
    mp.body.mode = BodyMode::Multipart;
    mp.body.multipart = vec![KeyValue::new("field", "value")];
    let r = send(&engine, ctx_for(mp, &base)).await.response.unwrap();
    let j: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert!(j["body"].as_str().unwrap().contains("name=\"field\""));

    // Cookie set by /login is replayed by the shared jar.
    let login = Request { url: "{{base}}/login".into(), ..Default::default() };
    let r = send(&engine, ctx_for(login, &base)).await.response.unwrap();
    assert_eq!(r.cookies[0].name, "sid");
    let echo = Request { url: "{{base}}/echo".into(), ..Default::default() };
    let r = send(&engine, ctx_for(echo, &base)).await.response.unwrap();
    let j: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(j["headers"]["cookie"], "sid=abc123");

    let redirect = Request { url: "{{base}}/redirect".into(), ..Default::default() };
    let r = send(&engine, ctx_for(redirect.clone(), &base)).await.response.unwrap();
    assert_eq!(r.status, 200);
    assert!(r.final_url.contains("from=redirect"));
    let mut no_follow = redirect;
    no_follow.settings.follow_redirects = false;
    let r = send(&engine, ctx_for(no_follow, &base)).await.response.unwrap();
    assert_eq!(r.status, 302);
}

#[tokio::test]
async fn runner_chains_variables_across_requests_and_iterations() {
    let base = http_server().await;
    let mut c = Collection::new("run");
    c.variables.push(KeyValue::new("base", &base));
    c.auth = Auth { kind: AuthKind::Bearer, bearer_token: "{{token}}".into(), ..Default::default() };
    let mut login = Request { name: "Login".into(), url: "{{base}}/login".into(), ..Default::default() };
    login.auth = Auth::none();
    login.scripts.test = r#"pm.collectionVariables.set("token", pm.response.json().token);"#.into();
    let mut me = Request { name: "Me".into(), url: "{{base}}/me?user={{user}}".into(), ..Default::default() };
    me.scripts.test = r#"
        const j = pm.response.json();
        pm.test("bearer from previous request", () => pm.expect(j.headers.authorization).to.equal("Bearer tok-42"));
        pm.test("data var", () => pm.expect(j.path).to.include("user=" + pm.iterationData.get("user")));
    "#
    .into();
    let mut missing = Request { name: "Missing".into(), url: "{{base}}/status/404".into(), ..Default::default() };
    missing.scripts.test = r#"pm.test("is 404", () => pm.response.to.be.notFound);"#.into();
    let mut folder = Folder::new("f");
    folder.items.push(Item::Request(me));
    c.items = vec![Item::Request(login), Item::Folder(folder), Item::Request(missing)];

    let data = tessera_core::runner::parse_data_file("user\nann\nbob\n").unwrap();
    let cfg = RunConfig {
        vars: ScriptVars { collection: vars_to_map(&c.variables), ..Default::default() },
        collection: c,
        folder_id: None,
        iterations: 0,
        delay_ms: 0,
        data,
        env_name: String::new(),
        settings: Settings::default(),
        certs: Vec::new(),
        stop_on_error: false,
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    run(Arc::new(HttpEngine::default()), cfg, Arc::new(AtomicBool::new(false)), move |e| sink.lock().unwrap().push(e)).await;
    let events = events.lock().unwrap();
    let results: Vec<_> = events.iter().filter_map(|e| if let RunEvent::Result(r) = e { Some(r.clone()) } else { None }).collect();
    assert_eq!(results.len(), 6, "2 iterations x 3 requests");
    for r in &results {
        assert!(r.tests.iter().all(|t| t.passed), "{}: {:?}", r.name, r.tests);
    }
    assert!(results[4].url.contains("user=bob"));
    let Some(RunEvent::Finished { vars, cancelled }) = events.last() else { panic!("no finish event") };
    assert!(!cancelled);
    assert_eq!(vars.collection.get("token").map(String::as_str), Some("tok-42"));
}

#[tokio::test]
async fn websocket_echo() {
    let url = ws_server().await;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let session = tokio::spawn(tessera_core::ws::run_session(url, vec![("X-Test".into(), "1".into())], true, rx, move |e| {
        sink.lock().unwrap().push(e)
    }));
    tx.send(tessera_core::ws::WsCommand::Send("hello".into())).unwrap();
    for _ in 0..50 {
        if events.lock().unwrap().iter().any(|e| matches!(e, tessera_core::ws::WsEvent::Message { incoming: true, .. })) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tx.send(tessera_core::ws::WsCommand::Close).unwrap();
    session.await.unwrap();
    let events = events.lock().unwrap();
    assert!(matches!(events[0], tessera_core::ws::WsEvent::Connected { status: 101, .. }), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, tessera_core::ws::WsEvent::Message { incoming: true, text, .. } if text == "echo: hello")));
    assert!(matches!(events.last(), Some(tessera_core::ws::WsEvent::Closed(_))));
}

/// Needs internet: `cargo test -p tessera-core --test integration -- --ignored`
#[tokio::test]
#[ignore]
async fn public_websocket_echo() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut req = Request::new(RequestKind::WebSocket);
    req.url = "wss://echo.websocket.org".into();
    let prep = tessera_core::http::prepare(&req, &Auth::none(), &Default::default(), &Settings::default());
    let headers: Vec<(String, String)> = prep.headers.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case("accept")).collect();
    eprintln!("headers: {headers:?} url: {}", prep.url);
    let session = tokio::spawn(tessera_core::ws::run_session(prep.url, headers, true, rx, move |e| {
        eprintln!("{e:?}");
        sink.lock().unwrap().push(e)
    }));
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tx.send(tessera_core::ws::WsCommand::Send("ping".into())).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    tx.send(tessera_core::ws::WsCommand::Close).unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), session).await;
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, tessera_core::ws::WsEvent::Connected { .. })));
}

#[tokio::test]
async fn flow_passes_token_between_requests_and_skips_after_failure() {
    use tessera_core::flow::*;
    let base = http_server().await;
    let mut c = Collection::new("api");
    c.variables.push(KeyValue::new("base", &base));
    let login = Request { name: "Login".into(), method: "POST".into(), url: "{{base}}/login".into(), ..Default::default() };
    let mut me = Request { name: "Me".into(), url: "{{base}}/me".into(), ..Default::default() };
    me.auth = Auth { kind: AuthKind::Bearer, bearer_token: "{{token}}".into(), ..Default::default() };
    let broken = Request { name: "Broken".into(), url: "{{base}}/status/404".into(), ..Default::default() };
    let after = Request { name: "After".into(), url: "{{base}}/after".into(), ..Default::default() };
    let ids: Vec<String> = [&login, &me, &broken, &after].iter().map(|r| r.id.clone()).collect();
    c.items = vec![Item::Request(login), Item::Request(me), Item::Request(broken), Item::Request(after)];

    let mut flow = Flow::new("login flow");
    for (i, rid) in ids.iter().enumerate() {
        flow.nodes.push(FlowNode { id: format!("n{i}"), collection_id: c.id.clone(), request_id: rid.clone(), pos: [i as f32 * 100.0, 0.0] });
    }
    let e = flow.connect("n0", "n1").unwrap();
    flow.edges.iter_mut().find(|x| x.id == e).unwrap().mappings.push(Mapping {
        source: MapSource::Body,
        path: "token".into(),
        variable: "token".into(),
        save_to_env: true,
    });
    flow.connect("n2", "n3").unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let cfg = FlowRunConfig {
        flow,
        collections: vec![c],
        vars: ScriptVars::default(),
        env_name: String::new(),
        settings: Settings::default(),
        certs: Vec::new(),
    };
    run_flow(Arc::new(HttpEngine::default()), cfg, Arc::new(AtomicBool::new(false)), move |e| sink.lock().unwrap().push(e)).await;
    let events = events.lock().unwrap();
    let result = |id: &str| {
        events
            .iter()
            .find_map(|e| match e {
                FlowEvent::NodeFinished(n, r) if n == id => Some(r.clone()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(result("n0").status, NodeStatus::Passed);
    assert_eq!(result("n0").extracted, vec![("token".to_string(), "tok-42".to_string())]);
    let me = result("n1");
    assert_eq!(me.status, NodeStatus::Passed);
    assert_eq!(me.json.unwrap()["headers"]["authorization"], "Bearer tok-42");
    assert_eq!(result("n2").status, NodeStatus::Failed);
    let skipped = result("n3");
    assert_eq!(skipped.status, NodeStatus::Skipped);
    assert!(skipped.error.unwrap().contains("Broken"));
    let Some(FlowEvent::Finished { vars, flow_vars, .. }) = events.last() else { panic!() };
    assert_eq!(vars.env.get("token").map(String::as_str), Some("tok-42"));
    assert_eq!(flow_vars.get("token").map(String::as_str), Some("tok-42"));
}
