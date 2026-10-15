//! cURL import plus code-snippet generation (cURL, PowerShell, Python, JavaScript).

use crate::http::{MultipartValue, PreparedBody, PreparedRequest};
use crate::model::{BodyMode, KeyValue, KvKind, RawLang, Request};

fn normalize(cmd: &str) -> String {
    let mut s = cmd
        .replace("\\\r\n", " ")
        .replace("\\\n", " ")
        .replace("^\r\n", " ")
        .replace("^\n", " ")
        .replace("`\r\n", " ")
        .replace("`\n", " ");
    // "Copy as cURL (cmd)" escapes with carets: ^" ^& ^% ...
    if s.contains("^\"") {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '^' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
        }
        s = out;
    }
    s
}

pub fn parse_curl(cmd: &str) -> Result<Request, String> {
    let args = shell_words::split(&normalize(cmd)).map_err(|e| format!("Could not parse cURL command: {e}"))?;
    let mut it = args.into_iter().peekable();
    match it.next() {
        Some(first) if first.eq_ignore_ascii_case("curl") || first.ends_with("curl.exe") => {}
        _ => return Err("Command must start with 'curl'".into()),
    }
    let mut method: Option<String> = None;
    let mut url = String::new();
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut form: Vec<KeyValue> = Vec::new();
    let mut get = false;
    let mut head = false;
    let mut user: Option<String> = None;
    let mut insecure = false;
    let mut timeout_ms = 0;
    let mut follow = false;

    const VALUE_SHORT: &[char] = &['X', 'H', 'd', 'u', 'F', 'b', 'A', 'e', 'o', 'm'];
    while let Some(arg) = it.next() {
        let (flag, inline): (String, Option<String>) = if arg.starts_with("--") {
            match arg.split_once('=') {
                Some((f, v)) if f.len() > 2 => (f.to_string(), Some(v.to_string())),
                _ => (arg.clone(), None),
            }
        } else if arg.starts_with('-') && arg.len() > 2 && arg.chars().nth(1).is_some_and(|c| VALUE_SHORT.contains(&c)) {
            (arg[..2].to_string(), Some(arg[2..].to_string()))
        } else {
            (arg.clone(), None)
        };
        let mut value = || inline.clone().or_else(|| it.next()).unwrap_or_default();
        match flag.as_str() {
            "-X" | "--request" => method = Some(value().to_uppercase()),
            "-H" | "--header" => {
                let h = value();
                if let Some((k, v)) = h.split_once(':') {
                    headers.push(KeyValue::new(k.trim(), v.trim()));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" | "--json" => {
                if flag == "--json" {
                    headers.push(KeyValue::new("Content-Type", "application/json"));
                }
                data.push(value())
            }
            "--data-urlencode" => {
                let v = value();
                data.push(match v.split_once('=') {
                    Some((k, val)) => format!("{k}={}", urlencoding::encode(val)),
                    None => urlencoding::encode(&v).into_owned(),
                });
            }
            "-F" | "--form" | "--form-string" => {
                let v = value();
                let (k, val) = v.split_once('=').unwrap_or((&v, ""));
                let mut kv = KeyValue::new(k, val);
                if flag != "--form-string" {
                    if let Some(path) = val.strip_prefix('@').or_else(|| val.strip_prefix('<')) {
                        kv.kind = KvKind::File;
                        kv.value = path.split(';').next().unwrap_or(path).to_string();
                    }
                }
                form.push(kv);
            }
            "-u" | "--user" => user = Some(value()),
            "-b" | "--cookie" => headers.push(KeyValue::new("Cookie", value())),
            "-A" | "--user-agent" => headers.push(KeyValue::new("User-Agent", value())),
            "-e" | "--referer" => headers.push(KeyValue::new("Referer", value())),
            "--oauth2-bearer" => headers.push(KeyValue::new("Authorization", format!("Bearer {}", value()))),
            "-m" | "--max-time" => timeout_ms = (value().parse::<f64>().unwrap_or(0.0) * 1000.0) as u64,
            "--url" => url = value(),
            "-o" | "--output" | "--connect-timeout" | "--retry" | "-w" | "--write-out" | "--proxy" | "-x" => {
                let _ = value();
            }
            "-G" | "--get" => get = true,
            "-I" | "--head" => head = true,
            "-k" | "--insecure" => insecure = true,
            "-L" | "--location" => follow = true,
            f if f.starts_with('-') => {}
            _ => {
                if url.is_empty() {
                    url = arg.clone();
                }
            }
        }
    }
    if url.is_empty() {
        return Err("No URL found in cURL command".into());
    }

    let mut req = Request::default();
    req.settings.verify_tls = !insecure;
    req.settings.follow_redirects = follow || req.settings.follow_redirects;
    req.settings.timeout_ms = timeout_ms;
    let joined = data.join("&");
    if get && !data.is_empty() {
        url = format!("{}{}{}", url, if url.contains('?') { '&' } else { '?' }, joined);
    }
    req.url = url;
    crate::vars::sync_params_from_url(&req.url.clone(), &mut req.params);

    let content_type = headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase());
    if !form.is_empty() {
        req.body.mode = BodyMode::Multipart;
        req.body.multipart = form;
        headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type") || !h.value.contains("multipart"));
    } else if !data.is_empty() && !get {
        if let Some(path) = joined.strip_prefix('@') {
            req.body.mode = BodyMode::Binary;
            req.body.binary_path = path.to_string();
        } else if content_type.as_deref().is_some_and(|c| c.contains("x-www-form-urlencoded"))
            || (content_type.is_none() && looks_like_form(&joined))
        {
            req.body.mode = BodyMode::FormUrlEncoded;
            req.body.form = url::form_urlencoded::parse(joined.as_bytes())
                .map(|(k, v)| KeyValue::new(k.into_owned(), v.into_owned()))
                .collect();
            headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
        } else {
            req.body.mode = BodyMode::Raw;
            req.body.raw = joined.clone();
            req.body.raw_lang = match content_type.as_deref() {
                Some(c) if c.contains("json") => RawLang::Json,
                Some(c) if c.contains("xml") => RawLang::Xml,
                Some(c) if c.contains("html") => RawLang::Html,
                _ if serde_json::from_str::<serde_json::Value>(&joined).is_ok() => RawLang::Json,
                _ => RawLang::Text,
            };
        }
    }
    if let Some(u) = user {
        let (name, pass) = u.split_once(':').unwrap_or((&u, ""));
        req.auth.kind = crate::model::AuthKind::Basic;
        req.auth.basic_user = name.into();
        req.auth.basic_pass = pass.into();
    } else {
        req.auth.kind = crate::model::AuthKind::None;
    }
    req.headers = headers;
    req.method = method.unwrap_or_else(|| {
        if head {
            "HEAD".into()
        } else if req.body.mode != BodyMode::None {
            "POST".into()
        } else {
            "GET".into()
        }
    });
    let path = url::Url::parse(&req.url).map(|u| u.path().to_string()).unwrap_or_else(|_| req.url.clone());
    req.name = format!("{} {}", req.method, path);
    Ok(req)
}

fn looks_like_form(s: &str) -> bool {
    !s.trim_start().starts_with(['{', '[', '<']) && s.split('&').all(|p| p.contains('=') && !p.contains(' '))
}

// ---------------------------------------------------------------- snippets

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnippetLang {
    Curl,
    PowerShell,
    Python,
    JavaScript,
}

impl SnippetLang {
    pub const ALL: [SnippetLang; 4] =
        [SnippetLang::Curl, SnippetLang::PowerShell, SnippetLang::Python, SnippetLang::JavaScript];
    pub fn label(self) -> &'static str {
        match self {
            SnippetLang::Curl => "cURL",
            SnippetLang::PowerShell => "PowerShell",
            SnippetLang::Python => "Python (requests)",
            SnippetLang::JavaScript => "JavaScript (fetch)",
        }
    }
}

fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// Headers worth showing in snippets (our own defaults are dropped).
fn snippet_headers(p: &PreparedRequest) -> Vec<(String, String)> {
    p.headers
        .iter()
        .filter(|(k, v)| {
            !(k.eq_ignore_ascii_case("user-agent") && v.starts_with("Tessera/"))
                && !(k.eq_ignore_ascii_case("accept") && v == "*/*")
        })
        .cloned()
        .collect()
}

pub fn snippet(p: &PreparedRequest, lang: SnippetLang) -> String {
    match lang {
        SnippetLang::Curl => to_curl(p),
        SnippetLang::PowerShell => to_powershell(p),
        SnippetLang::Python => to_python(p),
        SnippetLang::JavaScript => to_fetch(p),
    }
}

pub fn to_curl(p: &PreparedRequest) -> String {
    let mut parts = vec![format!("curl -X {} {}", p.method, sq(&p.url))];
    for (k, v) in snippet_headers(p) {
        parts.push(format!("-H {}", sq(&format!("{k}: {v}"))));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Text(t) => parts.push(format!("--data-raw {}", sq(t))),
        PreparedBody::Form(f) => {
            for (k, v) in f {
                parts.push(format!("--data-urlencode {}", sq(&format!("{k}={v}"))));
            }
        }
        PreparedBody::Multipart(f) => {
            for (k, v) in f {
                match v {
                    MultipartValue::Text(t) => parts.push(format!("-F {}", sq(&format!("{k}={t}")))),
                    MultipartValue::File(path) => parts.push(format!("-F {}", sq(&format!("{k}=@{path}")))),
                }
            }
        }
        PreparedBody::File(path) => parts.push(format!("--data-binary {}", sq(&format!("@{path}")))),
    }
    if !p.verify_tls {
        parts.push("--insecure".into());
    }
    if p.follow_redirects {
        parts.push("--location".into());
    }
    parts.join(" \\\n  ")
}

fn to_powershell(p: &PreparedRequest) -> String {
    let mut out = String::new();
    let headers = snippet_headers(p);
    let content_type = p.header("content-type").map(str::to_string);
    let headers: Vec<_> = headers.into_iter().filter(|(k, _)| !k.eq_ignore_ascii_case("content-type")).collect();
    if !headers.is_empty() {
        out.push_str("$headers = @{\n");
        for (k, v) in &headers {
            out.push_str(&format!("    {} = {}\n", ps(k), ps(v)));
        }
        out.push_str("}\n");
    }
    let mut call = format!("Invoke-RestMethod -Uri {} -Method {}", ps(&p.url), p.method);
    if !headers.is_empty() {
        call.push_str(" -Headers $headers");
    }
    if let Some(ct) = &content_type {
        call.push_str(&format!(" -ContentType {}", ps(ct)));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Text(t) => {
            out.push_str(&format!("$body = @'\n{t}\n'@\n"));
            call.push_str(" -Body $body");
        }
        PreparedBody::Form(f) => {
            out.push_str("$body = @{\n");
            for (k, v) in f {
                out.push_str(&format!("    {} = {}\n", ps(k), ps(v)));
            }
            out.push_str("}\n");
            call.push_str(" -Body $body");
        }
        PreparedBody::Multipart(f) => {
            out.push_str("$form = @{\n");
            for (k, v) in f {
                match v {
                    MultipartValue::Text(t) => out.push_str(&format!("    {} = {}\n", ps(k), ps(t))),
                    MultipartValue::File(path) => out.push_str(&format!("    {} = Get-Item {}\n", ps(k), ps(path))),
                }
            }
            out.push_str("}\n");
            call.push_str(" -Form $form");
        }
        PreparedBody::File(path) => call.push_str(&format!(" -InFile {}", ps(path))),
    }
    if !p.verify_tls {
        call.push_str(" -SkipCertificateCheck");
    }
    out.push_str(&call);
    out
}

fn to_python(p: &PreparedRequest) -> String {
    let mut out = String::from("import requests\n\n");
    out.push_str(&format!("url = {}\n", js(&p.url)));
    let headers = snippet_headers(p);
    if !headers.is_empty() {
        out.push_str("headers = {\n");
        for (k, v) in &headers {
            out.push_str(&format!("    {}: {},\n", js(k), js(v)));
        }
        out.push_str("}\n");
    }
    let mut args = vec![js(&p.method), "url".to_string()];
    if !headers.is_empty() {
        args.push("headers=headers".into());
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Text(t) => {
            out.push_str(&format!("payload = {}\n", js(t)));
            args.push("data=payload.encode(\"utf-8\")".into());
        }
        PreparedBody::Form(f) => {
            out.push_str("payload = {\n");
            for (k, v) in f {
                out.push_str(&format!("    {}: {},\n", js(k), js(v)));
            }
            out.push_str("}\n");
            args.push("data=payload".into());
        }
        PreparedBody::Multipart(f) => {
            out.push_str("data = {\n");
            for (k, v) in f {
                if let MultipartValue::Text(t) = v {
                    out.push_str(&format!("    {}: {},\n", js(k), js(t)));
                }
            }
            out.push_str("}\nfiles = {\n");
            for (k, v) in f {
                if let MultipartValue::File(path) = v {
                    out.push_str(&format!("    {}: open({}, \"rb\"),\n", js(k), js(path)));
                }
            }
            out.push_str("}\n");
            args.push("data=data".into());
            args.push("files=files".into());
        }
        PreparedBody::File(path) => {
            args.push(format!("data=open({}, \"rb\")", js(path)));
        }
    }
    if !p.verify_tls {
        args.push("verify=False".into());
    }
    if !p.follow_redirects {
        args.push("allow_redirects=False".into());
    }
    out.push_str(&format!("\nresponse = requests.request({})\nprint(response.status_code)\nprint(response.text)\n", args.join(", ")));
    out
}

fn to_fetch(p: &PreparedRequest) -> String {
    let mut out = String::new();
    let headers = snippet_headers(p);
    let mut opts = vec![format!("  method: {}", js(&p.method))];
    if !headers.is_empty() {
        let h: Vec<String> = headers.iter().map(|(k, v)| format!("    {}: {}", js(k), js(v))).collect();
        opts.push(format!("  headers: {{\n{}\n  }}", h.join(",\n")));
    }
    match &p.body {
        PreparedBody::None => {}
        PreparedBody::Text(t) => opts.push(format!("  body: {}", js(t))),
        PreparedBody::Form(f) => {
            let pairs: Vec<String> = f.iter().map(|(k, v)| format!("[{}, {}]", js(k), js(v))).collect();
            opts.push(format!("  body: new URLSearchParams([{}])", pairs.join(", ")));
        }
        PreparedBody::Multipart(f) => {
            out.push_str("const form = new FormData();\n");
            for (k, v) in f {
                match v {
                    MultipartValue::Text(t) => out.push_str(&format!("form.append({}, {});\n", js(k), js(t))),
                    MultipartValue::File(path) => {
                        out.push_str(&format!("form.append({}, fileInput.files[0]); // {}\n", js(k), path))
                    }
                }
            }
            opts.push("  body: form".into());
        }
        PreparedBody::File(path) => opts.push(format!("  body: fileBlob // contents of {path}")),
    }
    if !p.follow_redirects {
        opts.push("  redirect: \"manual\"".into());
    }
    out.push_str(&format!(
        "const response = await fetch({}, {{\n{}\n}});\nconsole.log(response.status, await response.text());\n",
        js(&p.url),
        opts.join(",\n")
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::prepare;
    use crate::model::{AuthKind, Settings};
    use crate::vars::VarScope;

    #[test]
    fn parses_chrome_style_curl() {
        let r = parse_curl(
            "curl 'https://api.x.io/v1/items?limit=5' \\\n  -H 'Authorization: Bearer abc' \\\n  -H 'Content-Type: application/json' \\\n  --data-raw '{\"a\":1}' --compressed",
        )
        .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://api.x.io/v1/items?limit=5");
        assert_eq!(r.params.len(), 1);
        assert_eq!(r.headers.len(), 2);
        assert_eq!(r.body.mode, BodyMode::Raw);
        assert_eq!(r.body.raw_lang, RawLang::Json);
    }

    #[test]
    fn parses_form_user_and_multipart() {
        let r = parse_curl("curl -u bob:pw -d a=1 -d b=2 https://x.io/login").unwrap();
        assert_eq!(r.body.mode, BodyMode::FormUrlEncoded);
        assert_eq!(r.body.form.len(), 2);
        assert_eq!(r.auth.kind, AuthKind::Basic);
        let m = parse_curl("curl -XPUT https://x.io/up -F file=@/tmp/a.png -F name=x").unwrap();
        assert_eq!(m.method, "PUT");
        assert_eq!(m.body.multipart[0].kind, KvKind::File);
        assert_eq!(m.body.multipart[0].value, "/tmp/a.png");
    }

    #[test]
    fn parses_windows_cmd_curl() {
        let r = parse_curl("curl ^\"https://x.io/a^\" ^\n  -H ^\"X-A: 1^\"").unwrap();
        assert_eq!(r.url, "https://x.io/a");
        assert_eq!(r.headers[0].key, "X-A");
    }

    #[test]
    fn curl_roundtrip() {
        let original = parse_curl("curl -X PATCH 'https://x.io/a?q=1' -H 'X-Y: it'\"'\"'s' --data-raw '{\"k\":\"v\"}'").unwrap();
        let settings = Settings { send_user_agent: false, ..Default::default() };
        let prep = prepare(&original, &original.auth, &VarScope::default(), &settings);
        let again = parse_curl(&to_curl(&prep)).unwrap();
        assert_eq!(again.method, "PATCH");
        assert_eq!(again.url, original.url);
        assert_eq!(again.body.raw, original.body.raw);
        assert!(again.headers.iter().any(|h| h.key == "X-Y" && h.value == "it's"));
        for lang in SnippetLang::ALL {
            assert!(!snippet(&prep, lang).is_empty());
        }
    }
}
