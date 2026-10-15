//! OAuth 2.0 token acquisition (authorization code + PKCE, client credentials, password, refresh).

use std::time::Duration;

use base64::Engine as _;
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::model::{OAuth2Config, OAuthGrant};
use crate::vars::VarScope;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub raw: String,
}

fn random_string(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

pub fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn resolved(cfg: &OAuth2Config, scope: &VarScope) -> OAuth2Config {
    let r = |s: &str| scope.resolve(s);
    OAuth2Config {
        auth_url: r(&cfg.auth_url),
        token_url: r(&cfg.token_url),
        client_id: r(&cfg.client_id),
        client_secret: r(&cfg.client_secret),
        scope: r(&cfg.scope),
        username: r(&cfg.username),
        password: r(&cfg.password),
        refresh_token: r(&cfg.refresh_token),
        ..cfg.clone()
    }
}

async fn token_request(cfg: &OAuth2Config, mut form: Vec<(&str, String)>) -> Result<OAuthToken, String> {
    if cfg.token_url.is_empty() {
        return Err("Token URL is required".into());
    }
    form.push(("client_id", cfg.client_id.clone()));
    if !cfg.client_secret.is_empty() {
        form.push(("client_secret", cfg.client_secret.clone()));
    }
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let resp = client
        .post(&cfg.token_url)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Token endpoint returned {status}: {text}"));
    }
    parse_token_response(&text)
}

pub fn parse_token_response(text: &str) -> Result<OAuthToken, String> {
    // Most servers send JSON; some (e.g. old GitHub) send form-encoded.
    let get = |key: &str| -> Option<String> {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            return v.get(key).map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string()));
        }
        url::form_urlencoded::parse(text.as_bytes()).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
    };
    let access_token = get("access_token").ok_or_else(|| format!("No access_token in response: {text}"))?;
    let expires_at = get("expires_in")
        .and_then(|s| s.parse::<i64>().ok())
        .map(|secs| chrono::Utc::now().timestamp() + secs)
        .unwrap_or(0);
    Ok(OAuthToken { access_token, refresh_token: get("refresh_token").unwrap_or_default(), expires_at, raw: text.into() })
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// Obtain a token for the configured grant. Authorization code opens the system browser
/// and waits (up to 5 minutes) for the redirect on a local loopback listener.
pub async fn fetch_token(cfg: &OAuth2Config, scope: &VarScope) -> Result<OAuthToken, String> {
    let cfg = resolved(cfg, scope);
    let scope_param = |form: &mut Vec<(&str, String)>| {
        if !cfg.scope.is_empty() {
            form.push(("scope", cfg.scope.clone()));
        }
    };
    match cfg.grant {
        OAuthGrant::ClientCredentials => {
            let mut form = vec![("grant_type", "client_credentials".to_string())];
            scope_param(&mut form);
            token_request(&cfg, form).await
        }
        OAuthGrant::Password => {
            let mut form = vec![
                ("grant_type", "password".to_string()),
                ("username", cfg.username.clone()),
                ("password", cfg.password.clone()),
            ];
            scope_param(&mut form);
            token_request(&cfg, form).await
        }
        OAuthGrant::AuthorizationCode => {
            if cfg.auth_url.is_empty() {
                return Err("Auth URL is required".into());
            }
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.redirect_port))
                .await
                .map_err(|e| format!("Cannot listen on port {}: {e}", cfg.redirect_port))?;
            let redirect = redirect_uri(cfg.redirect_port);
            let state = random_string(24);
            let verifier = random_string(64);
            let mut auth_url = url::Url::parse(&cfg.auth_url).map_err(|e| format!("Auth URL: {e}"))?;
            {
                let mut q = auth_url.query_pairs_mut();
                q.append_pair("response_type", "code");
                q.append_pair("client_id", &cfg.client_id);
                q.append_pair("redirect_uri", &redirect);
                q.append_pair("state", &state);
                if !cfg.scope.is_empty() {
                    q.append_pair("scope", &cfg.scope);
                }
                if cfg.use_pkce {
                    q.append_pair("code_challenge", &pkce_challenge(&verifier));
                    q.append_pair("code_challenge_method", "S256");
                }
            }
            open::that(auth_url.as_str()).map_err(|e| format!("Could not open browser: {e}"))?;
            let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state))
                .await
                .map_err(|_| "Timed out waiting for the browser redirect".to_string())??;
            let mut form = vec![
                ("grant_type", "authorization_code".to_string()),
                ("code", code),
                ("redirect_uri", redirect),
            ];
            if cfg.use_pkce {
                form.push(("code_verifier", verifier));
            }
            token_request(&cfg, form).await
        }
    }
}

pub async fn refresh_token(cfg: &OAuth2Config, scope: &VarScope) -> Result<OAuthToken, String> {
    let cfg = resolved(cfg, scope);
    if cfg.refresh_token.is_empty() {
        return Err("No refresh token available".into());
    }
    let mut tok = token_request(
        &cfg,
        vec![("grant_type", "refresh_token".to_string()), ("refresh_token", cfg.refresh_token.clone())],
    )
    .await?;
    if tok.refresh_token.is_empty() {
        tok.refresh_token = cfg.refresh_token.clone();
    }
    Ok(tok)
}

async fn wait_for_code(listener: &tokio::net::TcpListener, state: &str) -> Result<String, String> {
    loop {
        let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
        if !path.starts_with("/callback") {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let full = url::Url::parse(&format!("http://localhost{path}")).map_err(|e| e.to_string())?;
        let get = |k: &str| full.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned());
        let (ok, msg) = match (get("code"), get("error"), get("state")) {
            (_, Some(err), _) => (Err(format!("Authorization failed: {err} {}", get("error_description").unwrap_or_default())), "Authorization failed"),
            (Some(code), _, st) if st.as_deref() == Some(state) => (Ok(code), "Authorization complete"),
            (Some(_), _, _) => (Err("State mismatch in OAuth callback".to_string()), "State mismatch"),
            _ => (Err("No code in OAuth callback".to_string()), "No authorization code"),
        };
        let body = format!(
            "<html><body style=\"font-family:sans-serif;background:#16181d;color:#e6e6e6;display:flex;align-items:center;justify-content:center;height:100vh\"><div><h2>{msg}</h2><p>You can close this tab and return to Tessera.</p></div></body></html>"
        );
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        return ok;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn token_parsing() {
        let t = parse_token_response(r#"{"access_token":"a","refresh_token":"r","expires_in":3600}"#).unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_str()), ("a", "r"));
        assert!(t.expires_at > 0);
        let f = parse_token_response("access_token=x&token_type=bearer").unwrap();
        assert_eq!(f.access_token, "x");
    }
}
