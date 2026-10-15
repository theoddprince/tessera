//! Response body preprocessing (pretty printing, line splitting) done off the UI thread.

use tessera_core::http::HttpResponse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Empty,
    Json,
    Xml,
    Html,
    Text,
    Image,
    Binary,
}

impl BodyKind {
    pub fn label(self) -> &'static str {
        match self {
            BodyKind::Empty => "Empty",
            BodyKind::Json => "JSON",
            BodyKind::Xml => "XML",
            BodyKind::Html => "HTML",
            BodyKind::Text => "Text",
            BodyKind::Image => "Image",
            BodyKind::Binary => "Binary",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BodyView {
    pub kind: BodyKind,
    pub pretty: Vec<String>,
    pub raw: Vec<String>,
    pub json: Option<serde_json::Value>,
    pub image: Option<(String, Vec<u8>)>,
}

impl BodyView {
    pub fn empty() -> Self {
        Self { kind: BodyKind::Empty, pretty: Vec::new(), raw: Vec::new(), json: None, image: None }
    }
}

/// Lines longer than this are split for display so huge minified payloads never stall rendering.
const MAX_LINE: usize = 1500;
const MAX_TREE_BYTES: usize = 40 * 1024 * 1024;

fn split_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.len() <= MAX_LINE {
            out.push(line.to_string());
            continue;
        }
        let mut start = 0;
        while start < line.len() {
            let mut end = (start + MAX_LINE).min(line.len());
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            out.push(line[start..end].to_string());
            start = end;
        }
    }
    if out.is_empty() && !text.is_empty() {
        out.push(String::new());
    }
    out
}

fn is_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(4096)];
    sample.contains(&0) || std::str::from_utf8(sample).is_err() && String::from_utf8_lossy(sample).matches('\u{fffd}').count() > 8
}

pub fn build(resp: &HttpResponse, id: u64) -> BodyView {
    if resp.body.is_empty() {
        return BodyView::empty();
    }
    let ct = resp.content_type();
    if ct.starts_with("image/") && !ct.contains("svg") {
        let ext = ct.trim_start_matches("image/").split(';').next().unwrap_or("png").to_string();
        return BodyView {
            kind: BodyKind::Image,
            pretty: Vec::new(),
            raw: Vec::new(),
            json: None,
            image: Some((format!("bytes://response-{id}.{ext}"), resp.body.clone())),
        };
    }
    if is_binary(&resp.body) {
        let hex: Vec<String> = resp
            .body
            .chunks(16)
            .take(4096)
            .enumerate()
            .map(|(i, c)| {
                let h: Vec<String> = c.iter().map(|b| format!("{b:02x}")).collect();
                let a: String = c.iter().map(|&b| if (32..127).contains(&b) { b as char } else { '.' }).collect();
                format!("{:08x}  {:<48} {}", i * 16, h.join(" "), a)
            })
            .collect();
        return BodyView { kind: BodyKind::Binary, pretty: hex.clone(), raw: hex, json: None, image: None };
    }
    let text = resp.text();
    let trimmed = text.trim_start();
    let raw = split_lines(&text);
    let looks_json = ct.contains("json") || trimmed.starts_with('{') || trimmed.starts_with('[');
    if looks_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.clone());
            let json = (text.len() <= MAX_TREE_BYTES).then_some(v);
            return BodyView { kind: BodyKind::Json, pretty: split_lines(&pretty), raw, json, image: None };
        }
    }
    let kind = if ct.contains("html") || trimmed.to_ascii_lowercase().starts_with("<!doctype html") {
        BodyKind::Html
    } else if ct.contains("xml") || trimmed.starts_with("<?xml") || (trimmed.starts_with('<') && ct.is_empty()) {
        BodyKind::Xml
    } else {
        BodyKind::Text
    };
    let pretty = match kind {
        BodyKind::Xml | BodyKind::Html if text.lines().count() <= 3 && text.len() < 5_000_000 => {
            split_lines(&pretty_markup(&text))
        }
        _ => raw.clone(),
    };
    BodyView { kind, pretty, raw, json: None, image: None }
}

/// Minimal XML/HTML indenter for single-line markup.
pub fn pretty_markup(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + src.len() / 4);
    let mut depth: usize = 0;
    let mut i = 0;
    let bytes = src.as_bytes();
    let indent = |out: &mut String, d: usize| {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&"  ".repeat(d));
    };
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = src[i..].find('>').map(|e| i + e + 1).unwrap_or(bytes.len());
            let tag = &src[i..end];
            let closing = tag.starts_with("</");
            let self_closing = tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!");
            let void = {
                let name = tag.trim_start_matches('<').split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or("").to_ascii_lowercase();
                matches!(name.as_str(), "br" | "img" | "meta" | "link" | "input" | "hr")
            };
            if closing {
                depth = depth.saturating_sub(1);
            }
            // Keep `<a>text</a>` on one line.
            if closing && !out.ends_with('>') {
                out.push_str(tag);
            } else {
                indent(&mut out, depth);
                out.push_str(tag);
            }
            if !closing && !self_closing && !void {
                depth += 1;
            }
            i = end;
        } else {
            let end = src[i..].find('<').map(|e| i + e).unwrap_or(bytes.len());
            let text = src[i..end].trim();
            if !text.is_empty() {
                out.push_str(text);
            }
            i = end;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_xml() {
        assert_eq!(pretty_markup("<a><b>x</b><c/></a>"), "<a>\n  <b>x</b>\n  <c/>\n</a>");
    }

    #[test]
    fn long_lines_split() {
        let s = "é".repeat(2000);
        let lines = split_lines(&s);
        assert!(lines.len() >= 2);
        assert_eq!(lines.concat(), s);
    }

    #[test]
    fn json_detection() {
        let resp = HttpResponse { body: br#"{"a":[1,2]}"#.to_vec(), ..Default::default() };
        let v = build(&resp, 1);
        assert_eq!(v.kind, BodyKind::Json);
        assert_eq!(v.pretty.len(), 6);
    }
}
