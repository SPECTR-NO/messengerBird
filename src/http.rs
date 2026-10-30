//! Tiny VisionLab listener — POST /tx with JSON `{"message":"…"}`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

pub fn spawn(addr: &str) -> Result<(), String> {
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| e.to_string())?;
    eprintln!("MessengerBird listening on http://{addr}");
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                    thread::spawn(move || handle(stream));
                }
                Err(e) => eprintln!("http accept: {e}"),
            }
        }
    });
    Ok(())
}

fn handle(mut stream: TcpStream) {
    let mut buf = vec![0u8; 16_384];
    let n = match stream.read(&mut buf) {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let raw = String::from_utf8_lossy(&buf[..n]);
    let (method, path) = parse_request_line(&raw);
    let body = body_of(&raw);

    let (code, payload) = match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => (
            200,
            r#"{"ok":true,"app":"MessengerBird","hint":"desktop UI is the app; POST /tx to transmit"}"#.into(),
        ),
        ("GET", "/health") => (200, r#"{"ok":true}"#.into()),
        ("GET", "/api/status") => (200, status_json()),
        ("POST", "/tx") | ("POST", "/api/tx") | ("POST", "/webhook") => tx_response(&body),
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

fn body_of(raw: &str) -> String {
    if let Some(idx) = raw.find("\r\n\r\n") {
        return raw[idx + 4..].to_string();
    }
    if let Some(idx) = raw.find("\n\n") {
        return raw[idx + 2..].to_string();
    }
    String::new()
}

fn status_json() -> String {
    let lamp = match crate::radio::line_lamp() {
        crate::radio::LineLamp::Tx => "tx",
        crate::radio::LineLamp::Busy => "busy",
        crate::radio::LineLamp::Off => "idle",
    };
    serde_json::json!({
        "ok": true,
        "busy": crate::radio::busy(),
        "muted": crate::radio::muted(),
        "lamp": lamp,
        "last_message": crate::radio::last_message(),
    })
    .to_string()
}

fn tx_response(body: &str) -> (u16, String) {
    let message = extract_message(body).unwrap_or_else(|| "Alert".into());
    match crate::radio::transmit_message(&message) {
        Ok(()) => (
            200,
            serde_json::json!({
                "ok": true,
                "message": format!("Transmitting “{message}”"),
            })
            .to_string(),
        ),
        Err(e) => (
            503,
            serde_json::json!({
                "ok": false,
                "message": e,
            })
            .to_string(),
        ),
    }
}

fn extract_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    for key in ["message", "phrase", "name"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}
