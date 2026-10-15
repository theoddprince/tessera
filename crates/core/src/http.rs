//! Turning a `Request` into a wire request and executing it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::model::{
    ApiKeyIn, Auth, AuthKind, BodyMode, CertKind, ClientCert, KvKind, Request, RequestKind, Settings,
};
use crate::vars::VarScope;

pub const USER_AGENT: &str = concat!("Tessera/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub enum MultipartValue {
    Text(String),
    File(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreparedBody {
    None,
    Text(String),
    Form(Vec<(String, String)>),
    Multipart(Vec<(String, MultipartValue)>),
    File(String),
}

/// A fully resolved request (variables substituted, auth applied).
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: PreparedBody,
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub verify_tls: bool,
}

impl PreparedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn set_header_if_missing(&mut self, name: &str, value: impl Into<String>) {
        if self.header(name).is_none() {
            self.headers.push((name.to_string(), value.into()));
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: String,
    pub http_only: bool,
    pub secure: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub http_version: String,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<Cookie>,
    pub body: Vec<u8>,
    pub time_ms: f64,
    pub ttfb_ms: f64,
    pub final_url: String,
    pub request_method: String,
    pub request_url: String,
    pub request_headers: Vec<(String, String)>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn content_type(&self) -> String {
        self.header("content-type").unwrap_or("").to_ascii_lowercase()
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    /// Headers + body, roughly what went over the wire.
    pub fn size(&self) -> usize {
        self.body.len() + self.headers.iter().map(|(k, v)| k.len() + v.len() + 4).sum::<usize>()
    }
}

/// Apply resolved auth to a prepared request.
pub fn apply_auth(prep: &mut PreparedRequest, auth: &Auth, scope: &VarScope) {
    let r = |s: &str| scope.resolve(s);
    match auth.kind {
        AuthKind::Inherit | AuthKind::None => {}
        AuthKind::Bearer => {
            let t = r(&auth.bearer_token);
            if !t.is_empty() {
                prep.set_header_if_missing("Authorization", format!("Bearer {t}"));
            }
        }
        AuthKind::Basic => {
            let raw = format!("{}:{}", r(&auth.basic_user), r(&auth.basic_pass));
            let enc = base64::engine::general_purpose::STANDARD.encode(raw);
            prep.set_header_if_missing("Authorization", format!("Basic {enc}"));
        }
        AuthKind::ApiKey => {
            let (k, v) = (r(&auth.apikey_key), r(&auth.apikey_value));
            if k.is_empty() {
                return;
            }
            match auth.apikey_in {
                ApiKeyIn::Header => prep.set_header_if_missing(&k, v),
                ApiKeyIn::Query => {
                    let sep = if prep.url.contains('?') { '&' } else { '?' };
                    prep.url = format!("{}{}{}={}", prep.url, sep, urlencoding::encode(&k), urlencoding::encode(&v));
                }
            }
        }
        AuthKind::OAuth2 => {
            let t = r(&auth.oauth2.access_token);
            if !t.is_empty() {
                let prefix = auth.oauth2.header_prefix.trim();
                let value = if prefix.is_empty() { t } else { format!("{prefix} {t}") };
                prep.set_header_if_missing("Authorization", value);
            }
        }
    }
}

fn with_scheme(url: &str) -> String {
    let t = url.trim();
    if t.contains("://") {
        t.to_string()
    } else if t.is_empty() {
        String::new()
    } else {
        format!("http://{t}")
    }
}

/// Resolve variables, body and auth into something ready to send.
pub fn prepare(req: &Request, auth: &Auth, scope: &VarScope, settings: &Settings) -> PreparedRequest {
    let r = |s: &str| scope.resolve(s);
    let mut prep = PreparedRequest {
        method: if req.kind == RequestKind::GraphQL { "POST".into() } else { req.method.trim().to_uppercase() },
        url: with_scheme(&r(&req.url)),
        headers: req
            .headers
            .iter()
            .filter(|h| h.enabled && !h.key.trim().is_empty())
            .map(|h| (r(h.key.trim()), r(&h.value)))
            .collect(),
        body: PreparedBody::None,
        timeout_ms: if req.settings.timeout_ms > 0 { req.settings.timeout_ms } else { settings.timeout_ms },
        follow_redirects: req.settings.follow_redirects && settings.follow_redirects,
        verify_tls: req.settings.verify_tls && settings.verify_tls,
    };

    if req.kind == RequestKind::GraphQL {
        let vars_text = r(&req.body.graphql_variables);
        let variables: serde_json::Value = if vars_text.trim().is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_str(&vars_text).unwrap_or(serde_json::Value::String(vars_text))
        };
        let payload = serde_json::json!({ "query": r(&req.body.graphql_query), "variables": variables });
        prep.body = PreparedBody::Text(payload.to_string());
        prep.set_header_if_missing("Content-Type", "application/json");
    } else {
        match req.body.mode {
            BodyMode::None => {}
            BodyMode::Raw => {
                prep.body = PreparedBody::Text(r(&req.body.raw));
                prep.set_header_if_missing("Content-Type", req.body.raw_lang.content_type());
            }
            BodyMode::FormUrlEncoded => {
                prep.body = PreparedBody::Form(
                    req.body
                        .form
                        .iter()
                        .filter(|kv| kv.enabled && !kv.key.is_empty())
                        .map(|kv| (r(&kv.key), r(&kv.value)))
                        .collect(),
                );
            }
            BodyMode::Multipart => {
                prep.body = PreparedBody::Multipart(
                    req.body
                        .multipart
                        .iter()
                        .filter(|kv| kv.enabled && !kv.key.is_empty())
                        .map(|kv| {
                            let v = match kv.kind {
                                KvKind::Text => MultipartValue::Text(r(&kv.value)),
                                KvKind::File => MultipartValue::File(r(&kv.value)),
                            };
                            (r(&kv.key), v)
                        })
                        .collect(),
                );
            }
            BodyMode::Binary => {
                if !req.body.binary_path.is_empty() {
                    prep.body = PreparedBody::File(r(&req.body.binary_path));
                }
            }
        }
    }

    apply_auth(&mut prep, auth, scope);
    if settings.send_user_agent {
        prep.set_header_if_missing("User-Agent", USER_AGENT);
    }
    prep.set_header_if_missing("Accept", "*/*");
    prep
}

/// Shared HTTP machinery: client pool (per TLS/proxy/cert configuration) and a persistent cookie jar.
pub struct HttpEngine {
    clients: Mutex<HashMap<String, reqwest::Client>>,
    pub jar: Arc<reqwest::cookie::Jar>,
}

impl Default for HttpEngine {
    fn default() -> Self {
        Self { clients: Mutex::new(HashMap::new()), jar: Arc::new(reqwest::cookie::Jar::default()) }
    }
}

fn cert_for<'a>(certs: &'a [ClientCert], url: &url::Url) -> Option<&'a ClientCert> {
    let host = url.host_str()?;
    let with_port = format!("{}:{}", host, url.port_or_known_default().unwrap_or(0));
    certs.iter().filter(|c| c.enabled).find(|c| {
        let h = c.host.trim();
        h.eq_ignore_ascii_case(host)
            || h.eq_ignore_ascii_case(&with_port)
            || h.strip_prefix("*.").is_some_and(|suffix| host.ends_with(&format!(".{suffix}")))
    })
}

impl HttpEngine {
    pub fn client(
        &self,
        prep: &PreparedRequest,
        settings: &Settings,
        certs: &[ClientCert],
    ) -> Result<reqwest::Client, String> {
        let parsed = url::Url::parse(&prep.url).map_err(|e| format!("Invalid URL '{}': {e}", prep.url))?;
        let cert = cert_for(certs, &parsed);
        let key = format!(
            "{}|{}|{}|{}|{:?}",
            prep.verify_tls, prep.follow_redirects, settings.proxy, settings.ca_cert_path, cert
        );
        if let Some(c) = self.clients.lock().unwrap().get(&key) {
            return Ok(c.clone());
        }
        let mut b = reqwest::Client::builder()
            .cookie_provider(self.jar.clone())
            .danger_accept_invalid_certs(!prep.verify_tls)
            .redirect(if prep.follow_redirects {
                reqwest::redirect::Policy::limited(10)
            } else {
                reqwest::redirect::Policy::none()
            })
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90));
        if !settings.proxy.trim().is_empty() {
            b = b.proxy(reqwest::Proxy::all(settings.proxy.trim()).map_err(|e| format!("Proxy: {e}"))?);
        }
        if !settings.ca_cert_path.trim().is_empty() {
            let pem = std::fs::read(settings.ca_cert_path.trim()).map_err(|e| format!("CA certificate: {e}"))?;
            b = b.add_root_certificate(reqwest::Certificate::from_pem(&pem).map_err(|e| format!("CA certificate: {e}"))?);
        }
        if let Some(c) = cert {
            let identity = match c.kind {
                CertKind::Pem => {
                    let cert = std::fs::read(&c.cert_path).map_err(|e| format!("Client cert: {e}"))?;
                    let key = std::fs::read(&c.key_path).map_err(|e| format!("Client key: {e}"))?;
                    reqwest::Identity::from_pkcs8_pem(&cert, &key).map_err(|e| format!("Client cert: {e}"))?
                }
                CertKind::Pfx => {
                    let der = std::fs::read(&c.pfx_path).map_err(|e| format!("PFX: {e}"))?;
                    reqwest::Identity::from_pkcs12_der(&der, &c.passphrase).map_err(|e| format!("PFX: {e}"))?
                }
            };
            b = b.identity(identity);
        }
        let client = b.build().map_err(|e| format!("HTTP client: {e}"))?;
        self.clients.lock().unwrap().insert(key, client.clone());
        Ok(client)
    }

    /// Drop pooled clients (after settings / certificate changes).
    pub fn reset(&self) {
        self.clients.lock().unwrap().clear();
    }

    pub async fn execute(
        &self,
        prep: &PreparedRequest,
        settings: &Settings,
        certs: &[ClientCert],
    ) -> Result<HttpResponse, String> {
        if prep.url.is_empty() {
            return Err("Enter a URL first".into());
        }
        let client = self.client(prep, settings, certs)?;
        let method = reqwest::Method::from_bytes(prep.method.as_bytes()).map_err(|_| "Invalid method".to_string())?;
        let mut rb = client.request(method, &prep.url);
        if prep.timeout_ms > 0 {
            rb = rb.timeout(Duration::from_millis(prep.timeout_ms));
        }
        for (k, v) in &prep.headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        rb = match &prep.body {
            PreparedBody::None => rb,
            PreparedBody::Text(t) => rb.body(t.clone()),
            PreparedBody::Form(f) => rb.form(f),
            PreparedBody::File(path) => rb.body(std::fs::read(path).map_err(|e| format!("Body file '{path}': {e}"))?),
            PreparedBody::Multipart(fields) => {
                let mut form = reqwest::multipart::Form::new();
                for (k, v) in fields {
                    form = match v {
                        MultipartValue::Text(t) => form.text(k.clone(), t.clone()),
                        MultipartValue::File(path) => {
                            let bytes = std::fs::read(path).map_err(|e| format!("File '{path}': {e}"))?;
                            let name = std::path::Path::new(path)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "file".into());
                            form.part(k.clone(), reqwest::multipart::Part::bytes(bytes).file_name(name))
                        }
                    };
                }
                rb.multipart(form)
            }
        };

        let start = Instant::now();
        let resp = rb.send().await.map_err(describe_error)?;
        let ttfb = start.elapsed().as_secs_f64() * 1000.0;
        let status = resp.status();
        let http_version = format!("{:?}", resp.version());
        let final_url = resp.url().to_string();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
            .collect();
        let body = resp.bytes().await.map_err(describe_error)?.to_vec();
        let time_ms = start.elapsed().as_secs_f64() * 1000.0;
        let cookies = headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| parse_set_cookie(v))
            .collect();
        Ok(HttpResponse {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").to_string(),
            http_version,
            headers,
            cookies,
            body,
            time_ms,
            ttfb_ms: ttfb,
            final_url,
            request_method: prep.method.clone(),
            request_url: prep.url.clone(),
            request_headers: prep.headers.clone(),
        })
    }
}

fn describe_error(e: reqwest::Error) -> String {
    use std::error::Error;
    let mut msg = if e.is_timeout() {
        "Request timed out".to_string()
    } else if e.is_connect() {
        "Could not connect".to_string()
    } else {
        "Request failed".to_string()
    };
    let mut src: Option<&dyn Error> = e.source();
    let mut details = Vec::new();
    while let Some(s) = src {
        details.push(s.to_string());
        src = s.source();
    }
    if details.is_empty() {
        msg.push_str(&format!(": {e}"));
    } else {
        msg.push_str(&format!(": {}", details.join(" → ")));
    }
    msg
}

pub fn parse_set_cookie(raw: &str) -> Cookie {
    let mut parts = raw.split(';');
    let (name, value) = parts.next().and_then(|p| p.split_once('=')).unwrap_or((raw, ""));
    let mut c = Cookie { name: name.trim().into(), value: value.trim().into(), ..Default::default() };
    for attr in parts {
        let (k, v) = attr.split_once('=').unwrap_or((attr, ""));
        match k.trim().to_ascii_lowercase().as_str() {
            "domain" => c.domain = v.trim().into(),
            "path" => c.path = v.trim().into(),
            "expires" => c.expires = v.trim().into(),
            "max-age" if c.expires.is_empty() => c.expires = format!("max-age {}", v.trim()),
            "httponly" => c.http_only = true,
            "secure" => c.secure = true,
            _ => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyValue, RawLang};

    #[test]
    fn prepare_resolves_and_applies_auth() {
        let mut scope = VarScope::default();
        scope.environment.insert("host".into(), "example.com".into());
        scope.environment.insert("tok".into(), "abc".into());
        let mut req = Request { method: "post".into(), url: "{{host}}/x?a=1".into(), ..Default::default() };
        req.headers.push(KeyValue::new("X-Test", "{{tok}}"));
        req.body.mode = BodyMode::Raw;
        req.body.raw_lang = RawLang::Json;
        req.body.raw = r#"{"t":"{{tok}}"}"#.into();
        let auth = Auth { kind: AuthKind::Bearer, bearer_token: "{{tok}}".into(), ..Default::default() };
        let p = prepare(&req, &auth, &scope, &Settings::default());
        assert_eq!(p.method, "POST");
        assert_eq!(p.url, "http://example.com/x?a=1");
        assert_eq!(p.header("x-test"), Some("abc"));
        assert_eq!(p.header("authorization"), Some("Bearer abc"));
        assert_eq!(p.header("content-type"), Some("application/json"));
        assert_eq!(p.body, PreparedBody::Text(r#"{"t":"abc"}"#.into()));
    }

    #[test]
    fn api_key_in_query_and_basic() {
        let scope = VarScope::default();
        let mut p = PreparedRequest {
            method: "GET".into(),
            url: "https://x.io/a".into(),
            headers: vec![],
            body: PreparedBody::None,
            timeout_ms: 0,
            follow_redirects: true,
            verify_tls: true,
        };
        let auth = Auth {
            kind: AuthKind::ApiKey,
            apikey_key: "key".into(),
            apikey_value: "v 1".into(),
            apikey_in: ApiKeyIn::Query,
            ..Default::default()
        };
        apply_auth(&mut p, &auth, &scope);
        assert_eq!(p.url, "https://x.io/a?key=v%201");
        let basic = Auth { kind: AuthKind::Basic, basic_user: "u".into(), basic_pass: "p".into(), ..Default::default() };
        apply_auth(&mut p, &basic, &scope);
        assert_eq!(p.header("Authorization"), Some("Basic dTpw"));
    }

    #[test]
    fn set_cookie_parsing() {
        let c = parse_set_cookie("sid=123; Path=/; HttpOnly; Secure; Domain=.x.io");
        assert_eq!((c.name.as_str(), c.value.as_str(), c.path.as_str()), ("sid", "123", "/"));
        assert!(c.http_only && c.secure);
    }
}
