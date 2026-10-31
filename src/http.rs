//! One HTTP listener per sender config.
//! Each machine POSTs its own port: POST /tx plays that sender's fixed message.

use crate::config::SenderCfg;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn spawn_all(senders: &[SenderCfg]) -> Result<(), String> {
    if senders.is_empty() {
        return Err("no senders configured".into());
    }
    for sender in senders {
        spawn_one(sender.clone())?;
    }
    Ok(())
}

fn spawn_one(sender: SenderCfg) -> Result<(), String> {
    let addr = sender.listen.clone();
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "MessengerBird [{}] listening on http://{}/tx  →  “{}”",
        sender.id,
        addr,
        sender.phrase()
    );
    let sender = Arc::new(sender);
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
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

    let (code, payload) = match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => (
            200,
            serde_json::json!({
                "ok": true,
                "app": "MessengerBird",
                "id": sender.id,
                "name": sender.display_name(),
                "message": sender.phrase(),
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
        ("POST", "/tx") | ("POST", "/api/tx") | ("POST", "/webhook") => tx_response(sender),
        _ => (404, r#"{"ok":false,"message":"not found"}"#.into()),
    };

    let response = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{payload}",
        reason(code),
        payload.len(),
    );
    let _ = stream.write_all(response.as_bytes());
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
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
        "sender_muted": sender.muted,
        "busy": crate::radio::busy(),
        "muted": crate::radio::muted() || sender.muted,
        "lamp": lamp,
        "last_message": crate::radio::last_message(),
    })
    .to_string()
}

fn tx_response(sender: &SenderCfg) -> (u16, String) {
    if sender.muted {
        return (
            503,
            serde_json::json!({
                "ok": false,
                "id": sender.id,
                "message": format!("Sender “{}” is muted", sender.display_name()),
            })
            .to_string(),
        );
    }
    let message = sender.phrase().to_string();
    match crate::radio::transmit_message(&message) {
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
