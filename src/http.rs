//! One HTTP listener per sender config.
//! Each VisionLab sound POSTs its own port: POST /tx plays that sender's fixed message.
//! Binds `0.0.0.0:<port>` so Tailscale / LAN / WAN clients can reach the Pi.

use crate::config::SenderCfg;
use crate::netinfo::{force_public_bind, listen_port};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Shared optional token (`""` = auth off). Checked on POST /tx only; /health stays open for probes.
static AUTH_TOKEN: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn set_auth_token(token: &str) {
    if let Ok(mut g) = AUTH_TOKEN.lock() {
        *g = token.trim().to_string();
    }
}

fn auth_token() -> String {
    AUTH_TOKEN
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn spawn_all(senders: &[SenderCfg], auth_token: &str) -> Result<(), String> {
    set_auth_token(auth_token);
    if senders.is_empty() {
        return Err("no senders configured".into());
    }
    for sender in senders {
        spawn_one(sender.clone())?;
    }
    Ok(())
}

fn spawn_one(mut sender: SenderCfg) -> Result<(), String> {
    sender.listen = force_public_bind(&sender.listen);
    let addr = sender.listen.clone();
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "MessengerBird [{}] listening on http://0.0.0.0:{}/tx  →  “{}”",
        sender.id,
        listen_port(&addr),
        sender.phrase()
    );
    let sender = Arc::new(sender);
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(8)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(8)));
                    let sender = Arc::clone(&sender);
                    thread::spawn(move || handle(stream, &sender));
                }
                Err(e) => eprintln!("http accept [{}]: {e}", sender.id),
            }
        }
    });
    Ok(())
}

fn handle(mut stream: TcpStream, sender: &SenderCfg) {
    let mut buf = vec![0u8; 16_384];
    let n = match stream.read(&mut buf) {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let raw = String::from_utf8_lossy(&buf[..n]);
    let (method, path) = parse_request_line(&raw);
    let headers = parse_headers(&raw);

    let (code, payload) = match (method.as_str(), path.as_str()) {
        ("OPTIONS", _) => (204, String::new()),
        ("GET", "/") | ("GET", "/index.html") => (
            200,
            serde_json::json!({
                "ok": true,
                "app": "MessengerBird",
                "id": sender.id,
                "name": sender.display_name(),
                "message": sender.phrase(),
                "port": listen_port(&sender.listen),
                "hint": "POST /tx to transmit this sender's fixed announcement",
            })
            .to_string(),
        ),
        ("GET", "/health") => (
            200,
            serde_json::json!({
                "ok": true,
                "id": sender.id,
            })
            .to_string(),
        ),
        ("GET", "/api/status") => (200, status_json(sender)),
        ("POST", "/tx") | ("POST", "/api/tx") | ("POST", "/webhook") => {
            if let Some(err) = check_auth(&headers) {
                (
                    401,
                    serde_json::json!({
                        "ok": false,
                        "id": sender.id,
                        "message": err,
                    })
                    .to_string(),
                )
            } else {
                let peer = stream
                    .peer_addr()
                    .ok()
                    .map(|a| a.ip().to_string());
                tx_response(sender, peer)
            }
        }
        _ => (
            404,
            serde_json::json!({
                "ok": false,
                "id": sender.id,
                "message": "not found",
            })
            .to_string(),
        ),
    };

    let _ = write_response(&mut stream, code, &payload);
}

fn write_response(stream: &mut TcpStream, code: u16, payload: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\n\
Content-Type: application/json; charset=utf-8\r\n\
Content-Length: {len}\r\n\
Connection: close\r\n\
Cache-Control: no-store\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
Access-Control-Allow-Headers: Content-Type, X-MessengerBird-Token, Authorization\r\n\
\r\n\
{payload}",
        reason = reason(code),
        len = payload.len(),
    );
    stream.write_all(response.as_bytes())
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn parse_request_line(raw: &str) -> (String, String) {
    let line = raw.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts
        .next()
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();
    (method, path)
}

fn parse_headers(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in raw.lines().skip(1) {
        if line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    out
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn check_auth(headers: &[(String, String)]) -> Option<String> {
    let want = auth_token();
    if want.is_empty() {
        return None;
    }
    if let Some(got) = header_value(headers, "x-messengerbird-token") {
        if got == want {
            return None;
        }
    }
    if let Some(auth) = header_value(headers, "authorization") {
        let bearer = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            .unwrap_or("");
        if bearer == want {
            return None;
        }
    }
    Some("Unauthorized — set X-MessengerBird-Token (or Authorization: Bearer …)".into())
}

fn status_json(sender: &SenderCfg) -> String {
    let lamp = match crate::radio::line_lamp() {
        crate::radio::LineLamp::Tx => "tx",
        crate::radio::LineLamp::Busy => "busy",
        crate::radio::LineLamp::Off => "idle",
    };
    serde_json::json!({
        "ok": true,
        "id": sender.id,
        "name": sender.display_name(),
        "message": sender.phrase(),
        "listen": sender.listen,
        "port": listen_port(&sender.listen),
        "sender_muted": sender.muted,
        "busy": crate::radio::busy(),
        "muted": crate::radio::muted() || sender.muted,
        "lamp": lamp,
        "last_message": crate::radio::last_message(),
        "auth_required": !auth_token().is_empty(),
    })
    .to_string()
}

fn tx_response(sender: &SenderCfg, client_host: Option<String>) -> (u16, String) {
    if sender.muted {
        let msg = format!("Sender “{}” is muted", sender.display_name());
        crate::cloud::log_tx_simple(
            &sender.id,
            sender.phrase(),
            false,
            "http",
            client_host,
            Some(msg.clone()),
        );
        return (
            503,
            serde_json::json!({
                "ok": false,
                "id": sender.id,
                "message": msg,
            })
            .to_string(),
        );
    }
    if crate::radio::muted() {
        let msg = "Radio muted on MessengerBird".to_string();
        crate::cloud::log_tx_simple(
            &sender.id,
            sender.phrase(),
            false,
            "http",
            client_host,
            Some(msg.clone()),
        );
        return (
            503,
            serde_json::json!({
                "ok": false,
                "id": sender.id,
                "message": msg,
            })
            .to_string(),
        );
    }
    let message = sender.phrase().to_string();
    let meta = crate::radio::TxMeta {
        sender_id: sender.id.clone(),
        source: "http",
        client_host,
    };
    match crate::radio::transmit_with(&message, meta) {
        Ok(()) => (
            200,
            serde_json::json!({
                "ok": true,
                "id": sender.id,
                "message": format!("Transmitting “{message}”"),
            })
            .to_string(),
        ),
        Err(e) => (
            503,
            serde_json::json!({
                "ok": false,
                "id": sender.id,
                "message": e,
            })
            .to_string(),
        ),
    }
}
