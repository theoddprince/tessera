//! WebSocket sessions.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

#[derive(Clone, Debug)]
pub enum WsCommand {
    Send(String),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WsEvent {
    Connected { status: u16, headers: Vec<(String, String)> },
    Message { incoming: bool, text: String, binary: bool, at: String },
    Error(String),
    Closed(String),
}

/// Resolve and connect, IPv4 first with a short per-address timeout, so a dead IPv6
/// route doesn't stall the connection for the OS default (~20s).
async fn connect_tcp(uri: &tokio_tungstenite::tungstenite::http::Uri) -> Result<TcpStream, String> {
    let host = uri.host().ok_or("URL has no host")?.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("wss") { 443 } else { 80 });
    let mut addrs: Vec<SocketAddr> =
        tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| format!("Could not resolve {host}: {e}"))?.collect();
    addrs.sort_by_key(|a| a.is_ipv6());
    let mut last_err = format!("No addresses found for {host}");
    for addr in addrs {
        match tokio::time::timeout(Duration::from_secs(4), TcpStream::connect(addr)).await {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Ok(Err(e)) => last_err = format!("Could not connect to {addr}: {e}"),
            Err(_) => last_err = format!("Timed out connecting to {addr}"),
        }
    }
    Err(last_err)
}

fn now() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

pub async fn run_session(
    url: String,
    headers: Vec<(String, String)>,
    verify_tls: bool,
    mut cmds: UnboundedReceiver<WsCommand>,
    emit: impl Fn(WsEvent),
) {
    let mut request = match url.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => return emit(WsEvent::Error(format!("Invalid WebSocket URL: {e}"))),
    };
    for (k, v) in &headers {
        match (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
            (Ok(k), Ok(v)) => {
                request.headers_mut().insert(k, v);
            }
            _ => emit(WsEvent::Error(format!("Skipping invalid header '{k}'"))),
        }
    }
    let connector = match native_tls::TlsConnector::builder().danger_accept_invalid_certs(!verify_tls).build() {
        Ok(c) => Connector::NativeTls(c),
        Err(e) => return emit(WsEvent::Error(format!("TLS: {e}"))),
    };
    let tcp = match tokio::time::timeout(Duration::from_secs(20), connect_tcp(request.uri())).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return emit(WsEvent::Error(e)),
        Err(_) => return emit(WsEvent::Error("Connection timed out".into())),
    };
    let handshake = tokio_tungstenite::client_async_tls_with_config(request, tcp, None, Some(connector));
    let (stream, resp) = match tokio::time::timeout(Duration::from_secs(20), handshake).await {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return emit(WsEvent::Error(format!("Handshake failed: {e}"))),
        Err(_) => return emit(WsEvent::Error("Handshake timed out".into())),
    };
    emit(WsEvent::Connected {
        status: resp.status().as_u16(),
        headers: resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
            .collect(),
    });
    let (mut write, mut read) = stream.split();
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(WsCommand::Send(text)) => {
                    if let Err(e) = write.send(Message::Text(text.clone())).await {
                        emit(WsEvent::Error(format!("Send failed: {e}")));
                        break;
                    }
                    emit(WsEvent::Message { incoming: false, text, binary: false, at: now() });
                }
                Some(WsCommand::Close) | None => {
                    let _ = write.send(Message::Close(None)).await;
                    emit(WsEvent::Closed("Disconnected".into()));
                    break;
                }
            },
            msg = read.next() => match msg {
                Some(Ok(Message::Text(t))) => emit(WsEvent::Message { incoming: true, text: t.to_string(), binary: false, at: now() }),
                Some(Ok(Message::Binary(b))) => {
                    let preview: String = b.iter().take(256).map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ");
                    emit(WsEvent::Message { incoming: true, text: format!("[{} bytes] {preview}", b.len()), binary: true, at: now() });
                }
                Some(Ok(Message::Close(frame))) => {
                    let reason = frame.map(|f| format!("Closed by server: {} {}", u16::from(f.code), f.reason)).unwrap_or_else(|| "Closed by server".into());
                    emit(WsEvent::Closed(reason));
                    break;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    emit(WsEvent::Closed(format!("Connection error: {e}")));
                    break;
                }
                None => {
                    emit(WsEvent::Closed("Connection closed".into()));
                    break;
                }
            }
        }
    }
}
