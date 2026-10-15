//! `{{variable}}` resolution with Postman's precedence rules.

use std::collections::BTreeMap;
use std::ops::Range;

use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarSource {
    Local,
    Data,
    Environment,
    Collection,
    Global,
    Dynamic,
}

impl VarSource {
    pub fn label(self) -> &'static str {
        match self {
            VarSource::Local => "local",
            VarSource::Data => "iteration data",
            VarSource::Environment => "environment",
            VarSource::Collection => "collection",
            VarSource::Global => "global",
            VarSource::Dynamic => "dynamic",
        }
    }
}

/// Variable layers ordered from highest to lowest priority.
#[derive(Clone, Debug, Default)]
pub struct VarScope {
    pub local: BTreeMap<String, String>,
    pub data: BTreeMap<String, String>,
    pub environment: BTreeMap<String, String>,
    pub collection: BTreeMap<String, String>,
    pub globals: BTreeMap<String, String>,
}

impl VarScope {
    pub fn lookup(&self, name: &str) -> Option<(String, VarSource)> {
        let layers = [
            (&self.local, VarSource::Local),
            (&self.data, VarSource::Data),
            (&self.environment, VarSource::Environment),
            (&self.collection, VarSource::Collection),
            (&self.globals, VarSource::Global),
        ];
        for (map, src) in layers {
            if let Some(v) = map.get(name) {
                return Some((v.clone(), src));
            }
        }
        dynamic(name).map(|v| (v, VarSource::Dynamic))
    }

    /// Replace every `{{name}}` whose value is known. Values may themselves contain variables (bounded depth).
    pub fn resolve(&self, input: &str) -> String {
        let mut current = input.to_string();
        for _ in 0..5 {
            if !current.contains("{{") {
                break;
            }
            let next = self.resolve_once(&current);
            if next == current {
                break;
            }
            current = next;
        }
        current
    }

    fn resolve_once(&self, input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut last = 0;
        for (range, name) in find_vars(input) {
            out.push_str(&input[last..range.start]);
            match self.lookup(&name) {
                Some((v, _)) => out.push_str(&v),
                None => out.push_str(&input[range.clone()]),
            }
            last = range.end;
        }
        out.push_str(&input[last..]);
        out
    }

    /// Names referenced in `input` that have no value anywhere.
    pub fn missing(&self, input: &str) -> Vec<String> {
        find_vars(input).into_iter().map(|(_, n)| n).filter(|n| self.lookup(n).is_none()).collect()
    }
}

/// Byte ranges of `{{name}}` tokens (range covers the braces) and the trimmed names.
pub fn find_vars(input: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end_rel) = input[i + 2..].find("}}") {
                let name = input[i + 2..i + 2 + end_rel].trim();
                let end = i + 2 + end_rel + 2;
                if !name.is_empty() && !name.contains('{') && !name.contains('\n') {
                    out.push((i..end, name.to_string()));
                    i = end;
                    continue;
                }
            } else {
                break;
            }
        }
        i += 1;
    }
    out
}

fn dynamic(name: &str) -> Option<String> {
    let mut rng = rand::thread_rng();
    let v = match name {
        "$guid" | "$randomUUID" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => chrono::Utc::now().timestamp().to_string(),
        "$isoTimestamp" => chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "$randomInt" => rng.gen_range(0..1000).to_string(),
        "$randomBoolean" => rng.gen_bool(0.5).to_string(),
        "$randomAlphaNumeric" => {
            let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
            (chars[rng.gen_range(0..chars.len())] as char).to_string()
        }
        "$randomEmail" => format!("user{}@example.com", rng.gen_range(1000..99999)),
        "$randomFirstName" => ["Alex", "Sam", "Jordan", "Taylor", "Robin", "Kim"][rng.gen_range(0..6)].to_string(),
        _ => return None,
    };
    Some(v)
}

/// Split a URL into the part before `?` and decoded query pairs. Works textually so `{{vars}}` survive.
pub fn split_query(url: &str) -> (String, Vec<(String, String)>) {
    let (no_frag, _) = match url.find('#') {
        Some(p) => (&url[..p], &url[p..]),
        None => (url, ""),
    };
    match no_frag.find('?') {
        None => (url.to_string(), Vec::new()),
        Some(q) => {
            let base = url[..q].to_string();
            let pairs = no_frag[q + 1..]
                .split('&')
                .filter(|s| !s.is_empty())
                .map(|pair| match pair.split_once('=') {
                    Some((k, v)) => (k.to_string(), v.to_string()),
                    None => (pair.to_string(), String::new()),
                })
                .collect();
            (base, pairs)
        }
    }
}

/// Rebuild the URL from a base and the enabled params (values written as typed).
pub fn join_query(base: &str, params: &[crate::model::KeyValue]) -> String {
    let (base, _) = split_query(base);
    let q: Vec<String> = params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
        .map(|p| if p.value.is_empty() { p.key.clone() } else { format!("{}={}", p.key, p.value) })
        .collect();
    if q.is_empty() {
        base
    } else {
        format!("{}?{}", base, q.join("&"))
    }
}

/// Bring the params table in line with a freshly edited URL, keeping disabled rows and descriptions.
pub fn sync_params_from_url(url: &str, params: &mut Vec<crate::model::KeyValue>) {
    let (_, pairs) = split_query(url);
    let disabled: Vec<_> = params.iter().filter(|p| !p.enabled).cloned().collect();
    let old = std::mem::take(params);
    for (k, v) in pairs {
        let desc = old.iter().find(|p| p.enabled && p.key == k).map(|p| p.description.clone()).unwrap_or_default();
        params.push(crate::model::KeyValue { key: k, value: v, description: desc, ..Default::default() });
    }
    params.extend(disabled);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> VarScope {
        let mut s = VarScope::default();
        s.environment.insert("host".into(), "api.example.com".into());
        s.environment.insert("base".into(), "https://{{host}}".into());
        s.globals.insert("host".into(), "ignored".into());
        s.collection.insert("v".into(), "2".into());
        s
    }

    #[test]
    fn resolves_nested_with_precedence() {
        let s = scope();
        assert_eq!(s.resolve("{{base}}/v{{v}}/x"), "https://api.example.com/v2/x");
        assert_eq!(s.lookup("host").unwrap().1, VarSource::Environment);
    }

    #[test]
    fn keeps_missing_and_reports() {
        let s = scope();
        assert_eq!(s.resolve("{{nope}}/a"), "{{nope}}/a");
        assert_eq!(s.missing("{{nope}} {{ host }}"), vec!["nope".to_string()]);
    }

    #[test]
    fn dynamic_vars() {
        let s = VarScope::default();
        assert_eq!(s.resolve("{{$guid}}").len(), 36);
    }

    #[test]
    fn query_roundtrip() {
        let (base, q) = split_query("{{base}}/a?x=1&y={{v}}&flag");
        assert_eq!(base, "{{base}}/a");
        assert_eq!(q.len(), 3);
        let mut params = Vec::new();
        sync_params_from_url("{{base}}/a?x=1&y={{v}}&flag", &mut params);
        assert_eq!(join_query("{{base}}/a?old=1", &params), "{{base}}/a?x=1&y={{v}}&flag");
    }
}
