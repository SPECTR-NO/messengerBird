//! One HTTP listener per sender config.
//! Each VisionLab sound POSTs its own port: POST /tx plays that sender's fixed message.
//! Binds `0.0.0.0:<port>` so Tailscale / LAN / WAN clients can reach the Pi.

use crate::config::SenderCfg;
use crate::netinfo::{force_public_bind, listen_port};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Shared optional token (`""` = auth off). Checked on POST /tx only; /health stays open for probes.
static AUTH_TOKEN: Mutex<String> = Mutex::new(String::new());
static LIVE_SENDERS: Mutex<Vec<SenderCfg>> = Mutex::new(Vec::new());
static ALLOW_CIDRS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static RATE_PER_MIN: AtomicU32 = AtomicU32::new(30);
static RATE_HITS: LazyLock<Mutex<HashMap<String, (u32, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn set_auth_token(token: &str) {
    if let Ok(mut g) = AUTH_TOKEN.lock() {
        *g = token.trim().to_string();
    }
}

/// Push live sender mute/phrase and HTTP policy so listeners pick them up without a restart.
pub fn publish_runtime(senders: &[SenderCfg], rate_per_minute: u32, allow_cidrs: &[String]) {
    if let Ok(mut g) = LIVE_SENDERS.lock() {
        *g = senders.to_vec();
    }
    RATE_PER_MIN.store(rate_per_minute, Ordering::SeqCst);
    if let Ok(mut g) = ALLOW_CIDRS.lock() {
        *g = allow_cidrs.to_vec();
    }
}

fn live_sender(fallback: &SenderCfg) -> SenderCfg {
    LIVE_SENDERS
        .lock()
        .ok()
        .and_then(|g| g.iter().find(|s| s.id == fallback.id).cloned())
        .unwrap_or_else(|| fallback.clone())
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
    publish_runtime(senders, 30, &[]);
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

    let (code, payload, retry) = match (method.as_str(), path.as_str()) {
        ("OPTIONS", _) => (204, String::new(), None),
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
            None,
        ),
        ("GET", "/health") => (
            200,
            serde_json::json!({
                "ok": true,
                "id": sender.id,
            })
            .to_string(),
            None,
        ),
        ("GET", "/api/status") => (200, status_json(&live_sender(sender)), None),
        ("POST", "/tx") | ("POST", "/api/tx") | ("POST", "/webhook") => {
            let sender = live_sender(sender);
            if let Some(err) = check_auth(&headers) {
                (
                    401,
                    serde_json::json!({
                        "ok": false,
                        "id": sender.id,
                        "message": err,
                    })
                    .to_string(),
                    None,
                )
            } else {
                let peer = stream.peer_addr().ok().map(|a| a.ip().to_string());
                if let Some(ip) = peer.as_deref() {
                    if !ip_allowed(ip) {
                        (
                            403,
                            serde_json::json!({
                                "ok": false,
                                "id": sender.id,
                                "message": "Source IP is not allowlisted",
                            })
                            .to_string(),
                            None,
                        )
                    } else if !rate_ok(ip) {
                        (
                            429,
                            serde_json::json!({
                                "ok": false,
                                "id": sender.id,
                                "message": "Rate limit — retry shortly",
                            })
                            .to_string(),
                            Some(2u64),
                        )
                    } else {
                        let (code, body, retry) = tx_response(&sender, peer);
                        (code, body, retry)
                    }
                } else {
                    let (code, body, retry) = tx_response(&sender, peer);
                    (code, body, retry)
                }
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
            None,
        ),
    };

    let _ = write_response(&mut stream, code, &payload, retry);
}

fn write_response(
    stream: &mut TcpStream,
    code: u16,
    payload: &str,
    retry_after: Option<u64>,
) -> std::io::Result<()> {
    let retry = retry_after
        .map(|s| format!("Retry-After: {s}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\n\
Content-Type: application/json; charset=utf-8\r\n\
Content-Length: {len}\r\n\
Connection: close\r\n\
Cache-Control: no-store\r\n\
{retry}\
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
        202 => "Accepted",
        204 => "No Content",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
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
        "queue": crate::radio::queue_len(),
        "muted": crate::radio::muted() || sender.muted,
        "lamp": lamp,
        "last_message": crate::radio::last_message(),
        "auth_required": !auth_token().is_empty(),
    })
    .to_string()
}

fn tx_response(sender: &SenderCfg, client_host: Option<String>) -> (u16, String, Option<u64>) {
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
                "queued": false,
                "id": sender.id,
                "message": msg,
            })
            .to_string(),
            None,
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
                "queued": false,
                "id": sender.id,
                "message": msg,
            })
            .to_string(),
            None,
        );
    }
    let message = sender.phrase().to_string();
    let meta = crate::radio::TxMeta {
        sender_id: sender.id.clone(),
        source: "http",
        client_host,
    };
    match crate::radio::transmit_with(&message, meta) {
        Ok(crate::radio::TxAccept::Started) => (
            200,
            serde_json::json!({
                "ok": true,
                "queued": false,
                "id": sender.id,
                "message": format!("Transmitting “{message}”"),
            })
            .to_string(),
            None,
        ),
        Ok(crate::radio::TxAccept::Queued) => (
            202,
            serde_json::json!({
                "ok": true,
                "queued": true,
                "id": sender.id,
                "message": format!("Queued “{message}”"),
            })
            .to_string(),
            None,
        ),
        Err(e) => {
            let full = e.to_ascii_lowercase().contains("queue full");
            (
                503,
                serde_json::json!({
                    "ok": false,
                    "queued": false,
                    "id": sender.id,
                    "message": e,
                })
                .to_string(),
                if full { Some(2) } else { None },
            )
        }
    }
}

fn ip_allowed(ip: &str) -> bool {
    let cidrs = ALLOW_CIDRS.lock().ok();
    let Some(cidrs) = cidrs else {
        return true;
    };
    if cidrs.is_empty() {
        return true;
    }
    let Ok(addr) = ip.parse::<IpAddr>() else {
        return false;
    };
    cidrs.iter().any(|c| cidr_contains(c, addr))
}

fn cidr_contains(cidr: &str, ip: IpAddr) -> bool {
    let cidr = cidr.trim();
    if cidr.is_empty() {
        return false;
    }
    if cidr == ip.to_string() {
        return true;
    }
    let Some((net, bits)) = cidr.split_once('/') else {
        return false;
    };
    let Ok(bits) = bits.parse::<u32>() else {
        return false;
    };
    match (net.parse::<IpAddr>(), ip) {
        (Ok(IpAddr::V4(n)), IpAddr::V4(addr)) if bits <= 32 => {
            let mask = if bits == 0 {
                0
            } else {
                u32::MAX << (32 - bits)
            };
            (u32::from(n) & mask) == (u32::from(addr) & mask)
        }
        _ => false,
    }
}

fn rate_ok(ip: &str) -> bool {
    let limit = RATE_PER_MIN.load(Ordering::SeqCst);
    if limit == 0 {
        return true;
    }
    let Ok(mut map) = RATE_HITS.lock() else {
        return true;
    };
    let now = Instant::now();
    let entry = map.entry(ip.to_string()).or_insert((0, now));
    if entry.1.elapsed() >= Duration::from_secs(60) {
        *entry = (1, now);
        return true;
    }
    if entry.0 >= limit {
        return false;
    }
    entry.0 += 1;
    true
}

#[cfg(test)]
mod tests {
    use super::cidr_contains;
    use std::net::IpAddr;

    #[test]
    fn cidr_allows_tailscale_range() {
        let ip: IpAddr = "100.64.1.5".parse().unwrap();
        assert!(cidr_contains("100.64.0.0/10", ip));
        assert!(!cidr_contains("10.0.0.0/8", ip));
    }
}
