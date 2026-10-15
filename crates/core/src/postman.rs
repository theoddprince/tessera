//! Import and export of Postman collections (v2.0 / v2.1) and environments.

use serde_json::{json, Map, Value};

use crate::model::*;

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn description(v: &Value) -> String {
    match v {
        Value::Object(o) => o.get("content").map(s).unwrap_or_default(),
        other => s(other),
    }
}

fn kv_list(v: &Value) -> Vec<KeyValue> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let key = e.get("key").map(s)?;
                    Some(KeyValue {
                        key,
                        value: e.get("value").map(s).unwrap_or_default(),
                        enabled: !e.get("disabled").and_then(Value::as_bool).unwrap_or(false)
                            && e.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                        description: e.get("description").map(description).unwrap_or_default(),
                        kind: if e.get("type").and_then(Value::as_str) == Some("file") { KvKind::File } else { KvKind::Text },
                        secret: e.get("type").and_then(Value::as_str) == Some("secret"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn header_list(v: &Value) -> Vec<KeyValue> {
    if let Some(text) = v.as_str() {
        return text
            .lines()
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| KeyValue::new(k.trim(), v.trim()))
            .collect();
    }
    kv_list(v)
}

/// Reads `auth.<type>` in either v2.1 array form or v2.0 object form.
fn auth_params(auth: &Value, ty: &str) -> Map<String, Value> {
    match auth.get(ty) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|e| Some((e.get("key")?.as_str()?.to_string(), e.get("value").cloned().unwrap_or(Value::Null))))
            .collect(),
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    }
}

fn parse_auth(v: Option<&Value>, default: AuthKind) -> Auth {
    let Some(auth) = v.filter(|a| a.is_object()) else {
        return Auth { kind: default, ..Default::default() };
    };
    let ty = auth.get("type").and_then(Value::as_str).unwrap_or("noauth");
    let p = auth_params(auth, ty);
    let g = |k: &str| p.get(k).map(s).unwrap_or_default();
    let mut a = Auth::default();
    match ty {
        "bearer" => {
            a.kind = AuthKind::Bearer;
            a.bearer_token = g("token");
        }
        "basic" => {
            a.kind = AuthKind::Basic;
            a.basic_user = g("username");
            a.basic_pass = g("password");
        }
        "apikey" => {
            a.kind = AuthKind::ApiKey;
            a.apikey_key = g("key");
            a.apikey_value = g("value");
            a.apikey_in = if g("in") == "query" { ApiKeyIn::Query } else { ApiKeyIn::Header };
        }
        "oauth2" => {
            a.kind = AuthKind::OAuth2;
            let o = &mut a.oauth2;
            o.grant = match g("grant_type").as_str() {
                "client_credentials" => OAuthGrant::ClientCredentials,
                "password_credentials" | "password" => OAuthGrant::Password,
                _ => OAuthGrant::AuthorizationCode,
            };
            o.use_pkce = g("grant_type") == "authorization_code_with_pkce" || o.grant == OAuthGrant::AuthorizationCode;
            o.auth_url = g("authUrl");
            o.token_url = g("accessTokenUrl");
            o.client_id = g("clientId");
            o.client_secret = g("clientSecret");
            o.scope = g("scope");
            o.username = g("username");
            o.password = g("password");
            o.access_token = g("accessToken");
            let prefix = g("headerPrefix");
            if !prefix.is_empty() {
                o.header_prefix = prefix;
            }
        }
        "noauth" => a.kind = AuthKind::None,
        // Unsupported schemes (digest, hawk, aws, ...) fall back to inherit so nothing breaks silently.
        _ => a.kind = default,
    }
    a
}

fn parse_scripts(events: Option<&Value>) -> Scripts {
    let mut sc = Scripts::default();
    for ev in events.and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default() {
        let exec = &ev["script"]["exec"];
        let code = match exec {
            Value::Array(lines) => lines.iter().map(s).collect::<Vec<_>>().join("\n"),
            other => s(other),
        };
        match ev.get("listen").and_then(Value::as_str) {
            Some("prerequest") => sc.pre_request = code,
            Some("test") => sc.test = code,
            _ => {}
        }
    }
    sc
}

fn parse_url(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(u)) => u.clone(),
        Some(Value::Object(o)) => {
            if let Some(raw) = o.get("raw").and_then(Value::as_str) {
                return raw.to_string();
            }
            let protocol = o.get("protocol").map(s).unwrap_or_default();
            let host = match o.get("host") {
                Some(Value::Array(a)) => a.iter().map(s).collect::<Vec<_>>().join("."),
                Some(h) => s(h),
                None => String::new(),
            };
            let path = match o.get("path") {
                Some(Value::Array(a)) => a.iter().map(s).collect::<Vec<_>>().join("/"),
                Some(p) => s(p),
                None => String::new(),
            };
            let mut url = if protocol.is_empty() { host } else { format!("{protocol}://{host}") };
            if let Some(port) = o.get("port") {
                url.push_str(&format!(":{}", s(port)));
            }
            if !path.is_empty() {
                url.push('/');
                url.push_str(path.trim_start_matches('/'));
            }
            let q: Vec<String> = kv_list(o.get("query").unwrap_or(&Value::Null))
                .into_iter()
                .filter(|kv| kv.enabled)
                .map(|kv| format!("{}={}", kv.key, kv.value))
                .collect();
            if !q.is_empty() {
                url.push('?');
                url.push_str(&q.join("&"));
            }
            url
        }
        _ => String::new(),
    }
}

fn parse_request(item: &Value) -> Request {
    let r = &item["request"];
    let mut req = Request {
        name: item.get("name").map(s).unwrap_or_else(|| "Request".into()),
        method: r.get("method").map(s).filter(|m| !m.is_empty()).unwrap_or_else(|| "GET".into()).to_uppercase(),
        url: if r.is_string() { s(r) } else { parse_url(r.get("url")) },
        headers: header_list(r.get("header").unwrap_or(&Value::Null)),
        auth: parse_auth(r.get("auth"), AuthKind::Inherit),
        scripts: parse_scripts(item.get("event")),
        description: r.get("description").map(description).unwrap_or_default(),
        ..Default::default()
    };
    // Disabled query params live only in url.query.
    if let Some(q) = r.get("url").and_then(|u| u.get("query")) {
        req.params = kv_list(q);
    } else {
        crate::vars::sync_params_from_url(&req.url.clone(), &mut req.params);
    }
    if let Some(body) = r.get("body").filter(|b| b.is_object()) {
        let mode = body.get("mode").and_then(Value::as_str).unwrap_or("");
        match mode {
            "raw" => {
                req.body.mode = BodyMode::Raw;
                req.body.raw = body.get("raw").map(s).unwrap_or_default();
                let lang = body.pointer("/options/raw/language").and_then(Value::as_str).unwrap_or("");
                req.body.raw_lang = match lang {
                    "json" => RawLang::Json,
                    "xml" => RawLang::Xml,
                    "html" => RawLang::Html,
                    "javascript" => RawLang::JavaScript,
                    "text" => RawLang::Text,
                    _ => {
                        let ct = req.headers.iter().find(|h| h.key.eq_ignore_ascii_case("content-type"));
                        match ct {
                            Some(h) if h.value.contains("json") => RawLang::Json,
                            Some(h) if h.value.contains("xml") => RawLang::Xml,
                            _ if serde_json::from_str::<Value>(&req.body.raw).is_ok() => RawLang::Json,
                            _ => RawLang::Text,
                        }
                    }
                };
            }
            "urlencoded" => {
                req.body.mode = BodyMode::FormUrlEncoded;
                req.body.form = kv_list(body.get("urlencoded").unwrap_or(&Value::Null));
            }
            "formdata" => {
                req.body.mode = BodyMode::Multipart;
                let src = body.get("formdata").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default();
                req.body.multipart = kv_list(body.get("formdata").unwrap_or(&Value::Null));
                for (kv, raw) in req.body.multipart.iter_mut().zip(src.iter().filter(|e| e.get("key").is_some())) {
                    if kv.kind == KvKind::File {
                        kv.value = match raw.get("src") {
                            Some(Value::Array(a)) => a.first().map(s).unwrap_or_default(),
                            Some(v) => s(v),
                            None => String::new(),
                        };
                    }
                }
            }
            "file" => {
                req.body.mode = BodyMode::Binary;
                req.body.binary_path = body.pointer("/file/src").map(s).unwrap_or_default();
            }
            "graphql" => {
                req.kind = RequestKind::GraphQL;
                req.body.graphql_query = body.pointer("/graphql/query").map(s).unwrap_or_default();
                req.body.graphql_variables = body.pointer("/graphql/variables").map(s).unwrap_or_default();
            }
            _ => {}
        }
    }
    req
}

fn parse_items(items: Option<&Value>) -> Vec<Item> {
    items
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|it| {
            if it.get("item").is_some() {
                Item::Folder(Folder {
                    id: new_id(),
                    name: it.get("name").map(s).unwrap_or_else(|| "Folder".into()),
                    items: parse_items(it.get("item")),
                    auth: parse_auth(it.get("auth"), AuthKind::Inherit),
                    scripts: parse_scripts(it.get("event")),
                    description: it.get("description").map(description).unwrap_or_default(),
                })
            } else {
                Item::Request(parse_request(it))
            }
        })
        .collect()
}

pub fn import_collection(v: &Value) -> Result<Collection, String> {
    let info = v.get("info").ok_or("Not a Postman collection (missing 'info')")?;
    Ok(Collection {
        id: new_id(),
        name: info.get("name").map(s).unwrap_or_else(|| "Imported".into()),
        items: parse_items(v.get("item")),
        variables: kv_list(v.get("variable").unwrap_or(&Value::Null)),
        auth: parse_auth(v.get("auth"), AuthKind::None),
        scripts: parse_scripts(v.get("event")),
        description: info.get("description").map(description).unwrap_or_default(),
    })
}

pub fn import_environment(v: &Value) -> Result<Environment, String> {
    let values = v.get("values").ok_or("Not a Postman environment (missing 'values')")?;
    Ok(Environment {
        id: new_id(),
        name: v.get("name").map(s).unwrap_or_else(|| "Imported environment".into()),
        variables: kv_list(values),
    })
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Imported {
    Collection(Collection),
    Environment(Environment),
    Request(Request),
}

/// Detects the format (cURL, Postman collection/environment/dump, Tessera collection) and imports it.
pub fn import_any(text: &str) -> Result<Vec<Imported>, String> {
    let t = text.trim_start_matches('\u{feff}').trim();
    if t.starts_with("curl") {
        return crate::curl::parse_curl(t).map(|r| vec![Imported::Request(r)]);
    }
    let v: Value = serde_json::from_str(t).map_err(|e| format!("Not a cURL command or valid JSON: {e}"))?;
    if v.get("info").is_some() && v.get("item").is_some() {
        return Ok(vec![Imported::Collection(import_collection(&v)?)]);
    }
    if v.get("values").is_some() {
        return Ok(vec![Imported::Environment(import_environment(&v)?)]);
    }
    if v.get("collections").is_some() || v.get("environments").is_some() {
        // Postman bulk data dump (v1 collections aren't supported, but environments are).
        let mut out = Vec::new();
        for c in v.get("collections").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default() {
            if let Ok(col) = import_collection(c) {
                out.push(Imported::Collection(col));
            }
        }
        for e in v.get("environments").and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default() {
            if let Ok(env) = import_environment(e) {
                out.push(Imported::Environment(env));
            }
        }
        if out.is_empty() {
            return Err("Nothing importable found in the data dump".into());
        }
        return Ok(out);
    }
    if let Ok(mut c) = serde_json::from_value::<Collection>(v.clone()) {
        c.id = new_id();
        return Ok(vec![Imported::Collection(c)]);
    }
    Err("Unrecognised format. Supported: Postman collection v2.0/v2.1, Postman environment, cURL.".into())
}

// ---------------------------------------------------------------- export

fn kv_json(list: &[KeyValue]) -> Value {
    Value::Array(
        list.iter()
            .filter(|kv| !kv.is_blank())
            .map(|kv| {
                let mut o = json!({ "key": kv.key, "value": kv.value });
                if !kv.enabled {
                    o["disabled"] = json!(true);
                }
                if !kv.description.is_empty() {
                    o["description"] = json!(kv.description);
                }
                if kv.kind == KvKind::File {
                    o["type"] = json!("file");
                    o["src"] = json!(kv.value);
                    o.as_object_mut().unwrap().remove("value");
                } else {
                    o["type"] = json!("text");
                }
                o
            })
            .collect(),
    )
}

fn auth_json(a: &Auth) -> Option<Value> {
    let pairs = |ty: &str, items: Vec<(&str, String)>| {
        let arr: Vec<Value> = items.into_iter().map(|(k, v)| json!({ "key": k, "value": v, "type": "string" })).collect();
        json!({ "type": ty, ty: arr })
    };
    Some(match a.kind {
        AuthKind::Inherit => return None,
        AuthKind::None => json!({ "type": "noauth" }),
        AuthKind::Bearer => pairs("bearer", vec![("token", a.bearer_token.clone())]),
        AuthKind::Basic => pairs("basic", vec![("username", a.basic_user.clone()), ("password", a.basic_pass.clone())]),
        AuthKind::ApiKey => pairs(
            "apikey",
            vec![
                ("key", a.apikey_key.clone()),
                ("value", a.apikey_value.clone()),
                ("in", if a.apikey_in == ApiKeyIn::Query { "query" } else { "header" }.to_string()),
            ],
        ),
        AuthKind::OAuth2 => {
            let o = &a.oauth2;
            let grant = match o.grant {
                OAuthGrant::AuthorizationCode if o.use_pkce => "authorization_code_with_pkce",
                OAuthGrant::AuthorizationCode => "authorization_code",
                OAuthGrant::ClientCredentials => "client_credentials",
                OAuthGrant::Password => "password_credentials",
            };
            pairs(
                "oauth2",
                vec![
                    ("grant_type", grant.to_string()),
                    ("authUrl", o.auth_url.clone()),
                    ("accessTokenUrl", o.token_url.clone()),
                    ("clientId", o.client_id.clone()),
                    ("clientSecret", o.client_secret.clone()),
                    ("scope", o.scope.clone()),
                    ("username", o.username.clone()),
                    ("password", o.password.clone()),
                    ("headerPrefix", o.header_prefix.clone()),
                    ("redirect_uri", crate::oauth::redirect_uri(o.redirect_port)),
                    ("addTokenTo", "header".to_string()),
                ],
            )
        }
    })
}

fn events_json(sc: &Scripts) -> Value {
    let mut ev = Vec::new();
    for (listen, code) in [("prerequest", &sc.pre_request), ("test", &sc.test)] {
        if !code.trim().is_empty() {
            ev.push(json!({ "listen": listen, "script": { "type": "text/javascript", "exec": code.lines().collect::<Vec<_>>() } }));
        }
    }
    Value::Array(ev)
}

fn request_json(r: &Request) -> Value {
    let (base, _) = crate::vars::split_query(&r.url);
    let mut url = json!({ "raw": r.url, "query": kv_json(&r.params) });
    if let Some((proto, rest)) = base.split_once("://") {
        url["protocol"] = json!(proto);
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        url["host"] = json!(host.split('.').collect::<Vec<_>>());
        url["path"] = json!(path.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>());
    }
    let mut req = json!({
        "method": if r.kind == RequestKind::GraphQL { "POST" } else { r.method.as_str() },
        "header": kv_json(&r.headers),
        "url": url,
    });
    if !r.description.is_empty() {
        req["description"] = json!(r.description);
    }
    if let Some(a) = auth_json(&r.auth) {
        req["auth"] = a;
    }
    let body = if r.kind == RequestKind::GraphQL {
        Some(json!({ "mode": "graphql", "graphql": { "query": r.body.graphql_query, "variables": r.body.graphql_variables } }))
    } else {
        match r.body.mode {
            BodyMode::None => None,
            BodyMode::Raw => Some(json!({
                "mode": "raw", "raw": r.body.raw,
                "options": { "raw": { "language": r.body.raw_lang.label().to_lowercase() } }
            })),
            BodyMode::FormUrlEncoded => Some(json!({ "mode": "urlencoded", "urlencoded": kv_json(&r.body.form) })),
            BodyMode::Multipart => Some(json!({ "mode": "formdata", "formdata": kv_json(&r.body.multipart) })),
            BodyMode::Binary => Some(json!({ "mode": "file", "file": { "src": r.body.binary_path } })),
        }
    };
    if let Some(b) = body {
        req["body"] = b;
    }
    let mut item = json!({ "name": r.name, "request": req });
    let ev = events_json(&r.scripts);
    if ev.as_array().is_some_and(|a| !a.is_empty()) {
        item["event"] = ev;
    }
    item
}

fn items_json(items: &[Item]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| match i {
                Item::Request(r) => request_json(r),
                Item::Folder(f) => {
                    let mut o = json!({ "name": f.name, "item": items_json(&f.items) });
                    if let Some(a) = auth_json(&f.auth) {
                        o["auth"] = a;
                    }
                    let ev = events_json(&f.scripts);
                    if ev.as_array().is_some_and(|a| !a.is_empty()) {
                        o["event"] = ev;
                    }
                    if !f.description.is_empty() {
                        o["description"] = json!(f.description);
                    }
                    o
                }
            })
            .collect(),
    )
}

pub fn export_collection(c: &Collection) -> Value {
    let mut out = json!({
        "info": {
            "_postman_id": c.id,
            "name": c.name,
            "description": c.description,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "item": items_json(&c.items),
        "variable": kv_json(&c.variables),
    });
    if let Some(a) = auth_json(&c.auth) {
        out["auth"] = a;
    }
    let ev = events_json(&c.scripts);
    if ev.as_array().is_some_and(|a| !a.is_empty()) {
        out["event"] = ev;
    }
    out
}

pub fn export_environment(e: &Environment) -> Value {
    json!({
        "id": e.id,
        "name": e.name,
        "values": e.variables.iter().filter(|kv| !kv.key.is_empty()).map(|kv| json!({
            "key": kv.key, "value": kv.value, "enabled": kv.enabled,
            "type": if kv.secret { "secret" } else { "default" }
        })).collect::<Vec<_>>(),
        "_postman_variable_scope": "environment"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/sample.postman_collection.json");

    #[test]
    fn imports_sample_collection() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let c = import_collection(&v).unwrap();
        assert_eq!(c.name, "Sample API");
        assert_eq!(c.variables[0].key, "baseUrl");
        assert_eq!(c.auth.kind, AuthKind::Bearer);
        let Item::Folder(users) = &c.items[0] else { panic!("expected folder") };
        assert_eq!(users.items.len(), 2);
        let Item::Request(list) = &users.items[0] else { panic!() };
        assert_eq!(list.method, "GET");
        assert_eq!(list.url, "{{baseUrl}}/users?page=1");
        assert_eq!(list.params.len(), 2);
        assert!(!list.params[1].enabled);
        assert!(list.scripts.test.contains("pm.test"));
        let Item::Request(create) = &users.items[1] else { panic!() };
        assert_eq!(create.body.mode, BodyMode::Raw);
        assert_eq!(create.body.raw_lang, RawLang::Json);
        assert_eq!(create.auth.kind, AuthKind::Basic);
        let Item::Request(login) = &c.items[1] else { panic!() };
        assert_eq!(login.body.mode, BodyMode::FormUrlEncoded);
        assert_eq!(login.auth.kind, AuthKind::None);
        let Item::Request(gql) = &c.items[2] else { panic!() };
        assert_eq!(gql.kind, RequestKind::GraphQL);
    }

    #[test]
    fn export_then_reimport_is_stable() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let c = import_collection(&v).unwrap();
        let again = import_collection(&export_collection(&c)).unwrap();
        let strip = |c: &Collection| -> Vec<(String, String, String)> {
            c.all_requests().iter().map(|r| (r.name.clone(), r.method.clone(), r.url.clone())).collect()
        };
        assert_eq!(strip(&c), strip(&again));
        assert_eq!(again.auth, c.auth);
    }

    #[test]
    fn environment_roundtrip() {
        let env = import_environment(&json!({"name": "dev", "values": [
            {"key": "a", "value": "1", "enabled": true},
            {"key": "s", "value": "x", "enabled": false, "type": "secret"}
        ]}))
        .unwrap();
        assert!(!env.variables[1].enabled && env.variables[1].secret);
        let back = import_environment(&export_environment(&env)).unwrap();
        assert_eq!(back.variables, env.variables);
    }
}
