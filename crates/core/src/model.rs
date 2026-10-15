//! Data model shared by the engine, storage and UI.

use serde::{Deserialize, Serialize};

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn yes() -> bool {
    true
}

pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KvKind {
    #[default]
    Text,
    File,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub kind: KvKind,
    #[serde(default)]
    pub secret: bool,
}

impl Default for KeyValue {
    fn default() -> Self {
        Self {
            key: String::new(),
            value: String::new(),
            enabled: true,
            description: String::new(),
            kind: KvKind::Text,
            secret: false,
        }
    }
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: key.into(), value: value.into(), ..Default::default() }
    }
    pub fn is_blank(&self) -> bool {
        self.key.is_empty() && self.value.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestKind {
    #[default]
    Http,
    GraphQL,
    WebSocket,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyMode {
    #[default]
    None,
    Raw,
    FormUrlEncoded,
    Multipart,
    Binary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawLang {
    #[default]
    Json,
    Text,
    Xml,
    Html,
    JavaScript,
}

impl RawLang {
    pub const ALL: [RawLang; 5] = [RawLang::Json, RawLang::Text, RawLang::Xml, RawLang::Html, RawLang::JavaScript];
    pub fn label(self) -> &'static str {
        match self {
            RawLang::Json => "JSON",
            RawLang::Text => "Text",
            RawLang::Xml => "XML",
            RawLang::Html => "HTML",
            RawLang::JavaScript => "JavaScript",
        }
    }
    pub fn content_type(self) -> &'static str {
        match self {
            RawLang::Json => "application/json",
            RawLang::Text => "text/plain",
            RawLang::Xml => "application/xml",
            RawLang::Html => "text/html",
            RawLang::JavaScript => "application/javascript",
        }
    }
}

/// All body variants live side by side so switching modes in the UI never loses data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    pub mode: BodyMode,
    pub raw: String,
    pub raw_lang: RawLang,
    pub form: Vec<KeyValue>,
    pub multipart: Vec<KeyValue>,
    pub binary_path: String,
    pub graphql_query: String,
    pub graphql_variables: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthKind {
    #[default]
    Inherit,
    None,
    Bearer,
    Basic,
    ApiKey,
    OAuth2,
}

impl AuthKind {
    pub const ALL: [AuthKind; 6] =
        [AuthKind::Inherit, AuthKind::None, AuthKind::Bearer, AuthKind::Basic, AuthKind::ApiKey, AuthKind::OAuth2];
    pub fn label(self) -> &'static str {
        match self {
            AuthKind::Inherit => "Inherit from parent",
            AuthKind::None => "No auth",
            AuthKind::Bearer => "Bearer token",
            AuthKind::Basic => "Basic auth",
            AuthKind::ApiKey => "API key",
            AuthKind::OAuth2 => "OAuth 2.0",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiKeyIn {
    #[default]
    Header,
    Query,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OAuthGrant {
    #[default]
    AuthorizationCode,
    ClientCredentials,
    Password,
}

impl OAuthGrant {
    pub const ALL: [OAuthGrant; 3] =
        [OAuthGrant::AuthorizationCode, OAuthGrant::ClientCredentials, OAuthGrant::Password];
    pub fn label(self) -> &'static str {
        match self {
            OAuthGrant::AuthorizationCode => "Authorization Code (+PKCE)",
            OAuthGrant::ClientCredentials => "Client Credentials",
            OAuthGrant::Password => "Password Credentials",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OAuth2Config {
    pub grant: OAuthGrant,
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    pub username: String,
    pub password: String,
    pub redirect_port: u16,
    pub use_pkce: bool,
    pub header_prefix: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds; 0 = unknown.
    pub expires_at: i64,
}

impl Default for OAuth2Config {
    fn default() -> Self {
        Self {
            grant: OAuthGrant::AuthorizationCode,
            auth_url: String::new(),
            token_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            scope: String::new(),
            username: String::new(),
            password: String::new(),
            redirect_port: 7777,
            use_pkce: true,
            header_prefix: "Bearer".into(),
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    pub kind: AuthKind,
    pub bearer_token: String,
    pub basic_user: String,
    pub basic_pass: String,
    pub apikey_key: String,
    pub apikey_value: String,
    pub apikey_in: ApiKeyIn,
    pub oauth2: OAuth2Config,
}

impl Auth {
    pub fn none() -> Self {
        Self { kind: AuthKind::None, ..Default::default() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scripts {
    pub pre_request: String,
    pub test: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestSettings {
    /// 0 = use global default.
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub verify_tls: bool,
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self { timeout_ms: 0, follow_redirects: true, verify_tls: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    pub id: String,
    pub name: String,
    pub kind: RequestKind,
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body: Body,
    pub auth: Auth,
    pub scripts: Scripts,
    pub settings: RequestSettings,
    pub description: String,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Request".into(),
            kind: RequestKind::Http,
            method: "GET".into(),
            url: String::new(),
            params: Vec::new(),
            headers: Vec::new(),
            body: Body::default(),
            auth: Auth::default(),
            scripts: Scripts::default(),
            settings: RequestSettings::default(),
            description: String::new(),
        }
    }
}

impl Request {
    pub fn new(kind: RequestKind) -> Self {
        let mut r = Self { kind, ..Default::default() };
        match kind {
            RequestKind::GraphQL => {
                r.name = "New GraphQL".into();
                r.method = "POST".into();
            }
            RequestKind::WebSocket => r.name = "New WebSocket".into(),
            RequestKind::Http => {}
        }
        r
    }

    /// Short label shown in badges: GET / POST / GQL / WS.
    pub fn badge(&self) -> &str {
        match self.kind {
            RequestKind::Http => &self.method,
            RequestKind::GraphQL => "GQL",
            RequestKind::WebSocket => "WS",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub description: String,
}

impl Folder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            items: Vec::new(),
            auth: Auth::default(),
            scripts: Scripts::default(),
            description: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum Item {
    Folder(Folder),
    Request(Request),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Item::Folder(f) => &f.id,
            Item::Request(r) => &r.id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Item::Folder(f) => &f.name,
            Item::Request(r) => &r.name,
        }
    }
    pub fn contains_id(&self, id: &str) -> bool {
        if self.id() == id {
            return true;
        }
        match self {
            Item::Folder(f) => f.items.iter().any(|i| i.contains_id(id)),
            Item::Request(_) => false,
        }
    }
    /// Deep copy with fresh ids (used by "Duplicate").
    pub fn with_new_ids(&self) -> Item {
        match self {
            Item::Request(r) => Item::Request(Request { id: new_id(), ..r.clone() }),
            Item::Folder(f) => Item::Folder(Folder {
                id: new_id(),
                items: f.items.iter().map(|i| i.with_new_ids()).collect(),
                ..f.clone()
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub variables: Vec<KeyValue>,
    #[serde(default = "Auth::none")]
    pub auth: Auth,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub description: String,
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            items: Vec::new(),
            variables: Vec::new(),
            auth: Auth::none(),
            scripts: Scripts::default(),
            description: String::new(),
        }
    }

    pub fn find_request(&self, id: &str) -> Option<&Request> {
        find_request_in(&self.items, id)
    }
    pub fn find_request_mut(&mut self, id: &str) -> Option<&mut Request> {
        find_request_in_mut(&mut self.items, id)
    }

    /// Folders from the collection root down to (not including) the item `id`.
    pub fn folder_chain(&self, id: &str) -> Option<Vec<&Folder>> {
        fn walk<'a>(items: &'a [Item], id: &str, chain: &mut Vec<&'a Folder>) -> bool {
            for item in items {
                if item.id() == id {
                    return true;
                }
                if let Item::Folder(f) = item {
                    chain.push(f);
                    if walk(&f.items, id, chain) {
                        return true;
                    }
                    chain.pop();
                }
            }
            false
        }
        let mut chain = Vec::new();
        walk(&self.items, id, &mut chain).then_some(chain)
    }

    /// Auth that applies to a request after resolving "Inherit" through folders to the collection.
    pub fn effective_auth(&self, req: &Request) -> Auth {
        if req.auth.kind != AuthKind::Inherit {
            return req.auth.clone();
        }
        if let Some(chain) = self.folder_chain(&req.id) {
            for f in chain.iter().rev() {
                if f.auth.kind != AuthKind::Inherit {
                    return f.auth.clone();
                }
            }
        }
        if self.auth.kind == AuthKind::Inherit {
            Auth::none()
        } else {
            self.auth.clone()
        }
    }

    /// Scripts that run around a request, outermost first (collection, folders...). Excludes the request's own.
    pub fn parent_scripts(&self, req_id: &str) -> Vec<Scripts> {
        let mut out = vec![self.scripts.clone()];
        if let Some(chain) = self.folder_chain(req_id) {
            out.extend(chain.iter().map(|f| f.scripts.clone()));
        }
        out
    }

    pub fn all_requests(&self) -> Vec<&Request> {
        fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a Request>) {
            for i in items {
                match i {
                    Item::Request(r) => out.push(r),
                    Item::Folder(f) => walk(&f.items, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut out);
        out
    }

    /// Returns the item list of a container (the collection itself when `container_id == self.id`, or a folder).
    pub fn container_mut(&mut self, container_id: &str) -> Option<&mut Vec<Item>> {
        if container_id == self.id {
            return Some(&mut self.items);
        }
        find_folder_in_mut(&mut self.items, container_id).map(|f| &mut f.items)
    }

    pub fn find_folder_mut(&mut self, id: &str) -> Option<&mut Folder> {
        find_folder_in_mut(&mut self.items, id)
    }

    /// Removes an item anywhere in the tree and returns it.
    pub fn remove_item(&mut self, id: &str) -> Option<Item> {
        fn walk(items: &mut Vec<Item>, id: &str) -> Option<Item> {
            if let Some(pos) = items.iter().position(|i| i.id() == id) {
                return Some(items.remove(pos));
            }
            for i in items.iter_mut() {
                if let Item::Folder(f) = i {
                    if let Some(found) = walk(&mut f.items, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&mut self.items, id)
    }

    /// Container id and index of an item.
    pub fn locate(&self, id: &str) -> Option<(String, usize)> {
        fn walk(items: &[Item], parent: &str, id: &str) -> Option<(String, usize)> {
            for (idx, i) in items.iter().enumerate() {
                if i.id() == id {
                    return Some((parent.to_string(), idx));
                }
                if let Item::Folder(f) = i {
                    if let Some(found) = walk(&f.items, &f.id, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        walk(&self.items, &self.id, id)
    }

    pub fn contains_id(&self, id: &str) -> bool {
        self.id == id || self.items.iter().any(|i| i.contains_id(id))
    }
}

pub fn find_request_in<'a>(items: &'a [Item], id: &str) -> Option<&'a Request> {
    for i in items {
        match i {
            Item::Request(r) if r.id == id => return Some(r),
            Item::Folder(f) => {
                if let Some(r) = find_request_in(&f.items, id) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn find_request_in_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Request> {
    for i in items {
        match i {
            Item::Request(r) if r.id == id => return Some(r),
            Item::Folder(f) => {
                if let Some(r) = find_request_in_mut(&mut f.items, id) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_folder_in_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Folder> {
    for i in items {
        if let Item::Folder(f) = i {
            if f.id == id {
                return Some(f);
            }
            if let Some(found) = find_folder_in_mut(&mut f.items, id) {
                return Some(found);
            }
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub variables: Vec<KeyValue>,
}

impl Environment {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: new_id(), name: name.into(), variables: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub request: Request,
    pub status: u16,
    pub time_ms: f64,
    /// RFC 3339 local time.
    pub timestamp: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertKind {
    #[default]
    Pem,
    Pfx,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientCert {
    /// Host (optionally host:port) this certificate is presented to.
    pub host: String,
    pub kind: CertKind,
    pub cert_path: String,
    pub key_path: String,
    pub pfx_path: String,
    pub passphrase: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub verify_tls: bool,
    pub proxy: String,
    pub ca_cert_path: String,
    pub max_history: usize,
    pub dark_mode: bool,
    pub ui_scale: f32,
    pub send_user_agent: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            follow_redirects: true,
            verify_tls: true,
            proxy: String::new(),
            ca_cert_path: String::new(),
            max_history: 500,
            dark_mode: true,
            ui_scale: 1.0,
            send_user_agent: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
    pub globals: Vec<KeyValue>,
    pub active_env: Option<String>,
    pub history: Vec<HistoryEntry>,
    pub settings: Settings,
    pub certificates: Vec<ClientCert>,
    pub flows: Vec<crate::flow::Flow>,
}

impl Workspace {
    pub fn active_environment(&self) -> Option<&Environment> {
        let id = self.active_env.as_ref()?;
        self.environments.iter().find(|e| &e.id == id)
    }
    pub fn active_environment_mut(&mut self) -> Option<&mut Environment> {
        let id = self.active_env.clone()?;
        self.environments.iter_mut().find(|e| e.id == id)
    }
    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }
    pub fn collection_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.id == id)
    }
    /// Collection that owns the given item / folder / request id.
    pub fn collection_of(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.contains_id(id))
    }
    pub fn collection_of_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.contains_id(id))
    }

    pub fn push_history(&mut self, request: Request, status: u16, time_ms: f64) {
        self.history.insert(
            0,
            HistoryEntry {
                id: new_id(),
                request,
                status,
                time_ms,
                timestamp: chrono::Local::now().to_rfc3339(),
            },
        );
        let max = self.settings.max_history.max(1);
        self.history.truncate(max);
    }
}

/// Write a variable map back into an ordered key/value list: update existing, append new, drop unset.
pub fn merge_vars(list: &mut Vec<KeyValue>, map: &std::collections::BTreeMap<String, String>) {
    list.retain(|kv| !kv.enabled || kv.key.is_empty() || map.contains_key(&kv.key));
    for kv in list.iter_mut() {
        if let Some(v) = map.get(&kv.key) {
            if kv.enabled && &kv.value != v {
                kv.value = v.clone();
            }
        }
    }
    for (k, v) in map {
        if !list.iter().any(|kv| &kv.key == k && kv.enabled) {
            list.push(KeyValue::new(k.clone(), v.clone()));
        }
    }
}

pub fn vars_to_map(list: &[KeyValue]) -> std::collections::BTreeMap<String, String> {
    list.iter()
        .filter(|kv| kv.enabled && !kv.key.is_empty())
        .map(|kv| (kv.key.clone(), kv.value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherit_auth_walks_folders() {
        let mut c = Collection::new("c");
        c.auth = Auth { kind: AuthKind::Bearer, bearer_token: "root".into(), ..Default::default() };
        let mut f = Folder::new("f");
        f.auth = Auth { kind: AuthKind::Basic, basic_user: "u".into(), ..Default::default() };
        let r = Request::default();
        let rid = r.id.clone();
        f.items.push(Item::Request(r));
        c.items.push(Item::Folder(f));
        let req = c.find_request(&rid).unwrap().clone();
        assert_eq!(c.effective_auth(&req).kind, AuthKind::Basic);
        assert_eq!(c.parent_scripts(&rid).len(), 2);
    }

    #[test]
    fn merge_vars_updates_appends_removes() {
        let mut list = vec![KeyValue::new("a", "1"), KeyValue::new("b", "2")];
        let mut map = vars_to_map(&list);
        map.insert("a".into(), "9".into());
        map.remove("b");
        map.insert("c".into(), "3".into());
        merge_vars(&mut list, &map);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].value, "9");
        assert_eq!(list[1].key, "c");
    }
}
