//! Pre-request and test scripts, executed in an embedded JavaScript engine (Boa).

use std::collections::BTreeMap;

use boa_engine::{js_string, property::Attribute, Context, JsString, Source};
use serde::{Deserialize, Serialize};

const PRELUDE: &str = include_str!("script_prelude.js");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ScriptRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<Pair>,
    pub body: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResponse {
    pub code: u16,
    pub status: String,
    pub headers: Vec<Pair>,
    pub body: String,
    pub response_time: f64,
    pub response_size: usize,
    pub cookies: Vec<Pair>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptInfo {
    pub event_name: String,
    pub request_name: String,
    pub request_id: String,
    pub iteration: usize,
    pub iteration_count: usize,
}

/// Variable state that scripts can read and mutate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptVars {
    pub env: BTreeMap<String, String>,
    pub collection: BTreeMap<String, String>,
    pub globals: BTreeMap<String, String>,
    pub locals: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptInput<'a> {
    env: &'a BTreeMap<String, String>,
    collection: &'a BTreeMap<String, String>,
    globals: &'a BTreeMap<String, String>,
    locals: &'a BTreeMap<String, String>,
    data: &'a BTreeMap<String, String>,
    env_name: &'a str,
    request: &'a ScriptRequest,
    response: Option<&'a ScriptResponse>,
    info: &'a ScriptInfo,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsoleLine {
    pub level: String,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOutput {
    env: BTreeMap<String, String>,
    collection: BTreeMap<String, String>,
    globals: BTreeMap<String, String>,
    locals: BTreeMap<String, String>,
    tests: Vec<TestResult>,
    console: Vec<ConsoleLine>,
    #[serde(default)]
    next_request_set: bool,
    #[serde(default)]
    next_request: Option<String>,
    #[serde(default)]
    skip: bool,
    request: RawRequest,
}

#[derive(Deserialize)]
struct RawRequest {
    method: String,
    url: String,
    headers: Vec<Pair>,
}

#[derive(Clone, Debug, Default)]
pub struct ScriptOutcome {
    pub tests: Vec<TestResult>,
    pub console: Vec<ConsoleLine>,
    /// `Some(None)` = setNextRequest(null) (stop the run), `Some(Some(name))` = jump.
    pub next_request: Option<Option<String>>,
    pub skip_request: bool,
}

/// Runs a sequence of scripts (e.g. collection → folder → request) sharing one sandbox.
/// Mutates `vars` and `request` in place, mirroring what the scripts changed.
#[allow(clippy::too_many_arguments)]
pub fn run_scripts(
    scripts: &[&str],
    vars: &mut ScriptVars,
    data: &BTreeMap<String, String>,
    env_name: &str,
    request: &mut ScriptRequest,
    response: Option<&ScriptResponse>,
    info: &ScriptInfo,
) -> ScriptOutcome {
    let mut outcome = ScriptOutcome::default();
    let scripts: Vec<&str> = scripts.iter().copied().filter(|s| !s.trim().is_empty()).collect();
    if scripts.is_empty() {
        return outcome;
    }
    let input = ScriptInput {
        env: &vars.env,
        collection: &vars.collection,
        globals: &vars.globals,
        locals: &vars.locals,
        data,
        env_name,
        request,
        response,
        info,
    };
    let input_json = match serde_json::to_string(&input) {
        Ok(j) => j,
        Err(e) => {
            outcome.console.push(ConsoleLine { level: "error".into(), text: format!("Script input: {e}") });
            return outcome;
        }
    };

    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(50_000_000);
    ctx.runtime_limits_mut().set_recursion_limit(2_000);
    if let Err(e) = ctx.register_global_property(
        js_string!("__input_json"),
        JsString::from(input_json.as_str()),
        Attribute::all(),
    ) {
        outcome.console.push(ConsoleLine { level: "error".into(), text: format!("Sandbox: {e}") });
        return outcome;
    }
    if let Err(e) = ctx.eval(Source::from_bytes(PRELUDE.as_bytes())) {
        outcome.console.push(ConsoleLine { level: "error".into(), text: format!("Sandbox prelude: {e}") });
        return outcome;
    }
    for (i, code) in scripts.iter().enumerate() {
        // Each script runs in its own function scope so declarations don't collide.
        let wrapped = format!("(function(){{\n{code}\n}})();");
        if let Err(e) = ctx.eval(Source::from_bytes(wrapped.as_bytes())) {
            let label = if scripts.len() > 1 { format!(" (script {} of {})", i + 1, scripts.len()) } else { String::new() };
            outcome.console.push(ConsoleLine { level: "error".into(), text: format!("Script error{label}: {e}") });
        }
    }
    let result = ctx
        .eval(Source::from_bytes(b"__finish()"))
        .map_err(|e| e.to_string())
        .and_then(|v| v.to_string(&mut ctx).map_err(|e| e.to_string()))
        .map(|s| s.to_std_string_escaped());
    let raw: RawOutput = match result.and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())) {
        Ok(r) => r,
        Err(e) => {
            outcome.console.push(ConsoleLine { level: "error".into(), text: format!("Script result: {e}") });
            return outcome;
        }
    };

    vars.env = raw.env;
    vars.collection = raw.collection;
    vars.globals = raw.globals;
    vars.locals = raw.locals;
    request.method = raw.request.method;
    request.url = raw.request.url;
    request.headers = raw.request.headers;
    outcome.tests = raw.tests;
    outcome.console.extend(raw.console);
    outcome.next_request = raw.next_request_set.then_some(raw.next_request);
    outcome.skip_request = raw.skip;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &str, code: u16) -> ScriptResponse {
        ScriptResponse {
            code,
            status: "OK".into(),
            headers: vec![Pair { key: "Content-Type".into(), value: "application/json".into() }],
            body: body.into(),
            response_time: 12.0,
            response_size: body.len(),
            cookies: vec![Pair { key: "sid".into(), value: "1".into() }],
        }
    }

    #[test]
    fn tests_and_vars() {
        let mut vars = ScriptVars::default();
        vars.env.insert("base".into(), "x".into());
        let mut req = ScriptRequest { method: "GET".into(), url: "http://a/{{base}}".into(), ..Default::default() };
        let resp = response(r#"{"id":7,"tags":["a","b"],"user":{"name":"Ann"}}"#, 200);
        let script = r#"
            pm.test("status ok", function () { pm.response.to.have.status(200); });
            pm.test("is ok", function () { pm.response.to.be.ok; });
            pm.test("json", function () {
                var j = pm.response.json();
                pm.expect(j.id).to.equal(7);
                pm.expect(j.tags).to.be.an('array').that.includes('b');
                pm.expect(j).to.have.property('user');
                pm.expect(j.user).to.eql({name: 'Ann'});
                pm.expect(j.id).to.be.above(3).and.below(10);
                pm.expect(j.missing).to.not.exist;
            });
            pm.test("fails", function () { pm.expect(1).to.equal(2); });
            pm.test("header", function () { pm.response.to.have.header('content-type'); });
            pm.environment.set("token", pm.response.json().id);
            pm.collectionVariables.set("obj", {a: 1});
            pm.environment.unset("base");
            console.log("hello", {a: 1});
        "#;
        let out = run_scripts(&[script], &mut vars, &BTreeMap::new(), "dev", &mut req, Some(&resp), &ScriptInfo::default());
        let summary: Vec<(String, bool)> = out.tests.iter().map(|t| (t.name.clone(), t.passed)).collect();
        assert_eq!(
            summary,
            vec![
                ("status ok".into(), true),
                ("is ok".into(), true),
                ("json".into(), true),
                ("fails".into(), false),
                ("header".into(), true)
            ],
            "{:?}",
            out
        );
        assert!(out.tests[3].error.contains("expected 1 to equal 2"));
        assert_eq!(vars.env.get("token").map(String::as_str), Some("7"));
        assert!(!vars.env.contains_key("base"));
        assert_eq!(vars.collection.get("obj").map(String::as_str), Some(r#"{"a":1}"#));
        assert!(out.console[0].text.starts_with("hello"));
    }

    #[test]
    fn pre_request_mutates_request_and_reports_errors() {
        let mut vars = ScriptVars::default();
        let mut req = ScriptRequest { method: "GET".into(), url: "http://a".into(), ..Default::default() };
        let scripts = [
            "pm.request.headers.add({key: 'X-Sig', value: btoa('abc')}); pm.variables.set('n', 1);",
            "this is not javascript",
            "pm.request.url.update(pm.request.url.toString() + '/b?n=' + pm.variables.get('n')); pm.setNextRequest(null);",
        ];
        let out = run_scripts(&scripts, &mut vars, &BTreeMap::new(), "", &mut req, None, &ScriptInfo::default());
        assert_eq!(req.headers, vec![Pair { key: "X-Sig".into(), value: "YWJj".into() }]);
        assert_eq!(req.url, "http://a/b?n=1");
        assert_eq!(out.next_request, Some(None));
        assert!(out.console.iter().any(|l| l.level == "error"));
    }

    #[test]
    fn infinite_loop_is_stopped() {
        let mut vars = ScriptVars::default();
        let mut req = ScriptRequest::default();
        let out = run_scripts(&["while(true){}"], &mut vars, &BTreeMap::new(), "", &mut req, None, &ScriptInfo::default());
        assert!(out.console.iter().any(|l| l.level == "error"));
    }
}
