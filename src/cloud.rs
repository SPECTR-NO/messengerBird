//! Lightweight Supabase Auth + PostgREST client for MessengerBird cloud.
//! Offline-first: missing config / unsigned-in → no-ops; radio + HTTP keep working.

use crate::config::{Config, SenderCfg};
use crate::netinfo::{hostname, primary_ipv4, tailscale_ipv4};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CloudConfig {
    pub url: String,
    pub publishable_key: String,
    #[serde(default = "default_display_name")]
    pub display_name: String,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u64,
}

fn default_display_name() -> String {
    "MessengerBird".into()
}
fn default_sync_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct SessionFile {
    access_token: String,
    refresh_token: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    expires_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct CloudStatus {
    pub configured: bool,
    pub signed_in: bool,
    pub email: String,
    pub last_sync: Option<u64>,
    pub last_error: Option<String>,
    pub syncing: bool,
}

struct State {
    cfg: Option<CloudConfig>,
    session: SessionFile,
    status: CloudStatus,
    last_heartbeat: Option<Instant>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            cfg: None,
            session: SessionFile::default(),
            status: CloudStatus::default(),
            last_heartbeat: None,
        })
    })
}

pub fn config_path() -> PathBuf {
    locate_beside_app("config/supabase.yaml")
}

pub fn session_path() -> PathBuf {
    locate_beside_app("config/supabase.session.yaml")
}

fn locate_beside_app(rel: &str) -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join(rel);
        if p.exists() || cwd.join("config.yaml").exists() {
            return p;
        }
        let p2 = cwd.join("config").join(
            Path::new(rel)
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("supabase.yaml")),
        );
        if cwd.join("Cargo.toml").exists() {
            return p2;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(rel);
            if p.exists() {
                return p;
            }
            if let Some(root) = exe.ancestors().nth(3) {
                let p = root.join(rel);
                if p.exists() || root.join("config.yaml").exists() {
                    return p;
                }
            }
        }
    }
    PathBuf::from(rel)
}

pub fn init() {
    let path = config_path();
    let cfg = load_cloud_config(&path);
    let session = load_session(&session_path());
    if let Ok(mut g) = state().lock() {
        g.status.configured = cfg.is_some();
        if let Some(ref c) = cfg {
            if c.url.trim().is_empty() || c.publishable_key.trim().is_empty() {
                g.status.configured = false;
            }
        }
        g.cfg = cfg;
        g.session = session.clone();
        g.status.signed_in = !session.access_token.is_empty();
        g.status.email = session.email;
        if g.status.signed_in {
            g.status.last_error = None;
        }
    }
}

fn load_cloud_config(path: &Path) -> Option<CloudConfig> {
    let text = fs::read_to_string(path).ok()?;
    let mut cfg: CloudConfig = serde_yaml::from_str(&text).ok()?;
    cfg.url = cfg.url.trim().trim_end_matches('/').to_string();
    cfg.publishable_key = cfg.publishable_key.trim().to_string();
    if cfg.url.is_empty() || cfg.publishable_key.is_empty() {
        return None;
    }
    if cfg.url.contains("YOUR_PROJECT") || cfg.publishable_key.contains("YOUR_KEY") {
        return None;
    }
    Some(cfg)
}

fn load_session(path: &Path) -> SessionFile {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_yaml::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_session(session: &SessionFile) -> Result<(), String> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let text = serde_yaml::to_string(session).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}

pub fn status() -> CloudStatus {
    state()
        .lock()
        .ok()
        .map(|g| g.status.clone())
        .unwrap_or_default()
}

pub fn sign_in(email: &str, password: &str) -> Result<(), String> {
    let (url, key) = with_cfg(|c| (c.url.clone(), c.publishable_key.clone()))?;
    let body = serde_json::json!({
        "email": email.trim(),
        "password": password,
    });
    let resp = http_json(
        "POST",
        &format!("{url}/auth/v1/token?grant_type=password"),
        &key,
        None,
        Some(&body),
    )?;
    apply_token_response(&resp, email.trim())?;
    Ok(())
}

pub fn sign_up(email: &str, password: &str) -> Result<(), String> {
    let (url, key) = with_cfg(|c| (c.url.clone(), c.publishable_key.clone()))?;
    let body = serde_json::json!({
        "email": email.trim(),
        "password": password,
    });
    let resp = http_json(
        "POST",
        &format!("{url}/auth/v1/signup"),
        &key,
        None,
        Some(&body),
    )?;
    // Signup may require email confirm — try to capture session if present.
    if resp.get("access_token").and_then(|v| v.as_str()).is_some() {
        apply_token_response(&resp, email.trim())?;
        Ok(())
    } else {
        Err("Check your email to confirm, then sign in.".into())
    }
}

pub fn sign_out() {
    if let Some((url, key, token)) = state().lock().ok().and_then(|g| {
        let cfg = g.cfg.as_ref()?;
        if g.session.access_token.is_empty() {
            None
        } else {
            Some((
                cfg.url.clone(),
                cfg.publishable_key.clone(),
                g.session.access_token.clone(),
            ))
        }
    }) {
        let _ = http_json(
            "POST",
            &format!("{url}/auth/v1/logout?scope=global"),
            &key,
            Some(&token),
            None,
        );
    }
    if let Ok(mut g) = state().lock() {
        g.session = SessionFile::default();
        g.status.signed_in = false;
        g.status.email.clear();
        g.status.last_error = None;
    }
    let _ = fs::remove_file(session_path());
}

pub fn request_password_reset(email: &str) -> Result<(), String> {
    let email = email.trim();
    if email.is_empty() {
        return Err("Enter the account email first.".into());
    }
    let (url, key) = with_cfg(|c| (c.url.clone(), c.publishable_key.clone()))?;
    let body = serde_json::json!({ "email": email });
    http_json(
        "POST",
        &format!("{url}/auth/v1/recover"),
        &key,
        None,
        Some(&body),
    )?;
    Ok(())
}

fn apply_token_response(resp: &serde_json::Value, email_fallback: &str) -> Result<(), String> {
    let access = resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| auth_err_message(resp))?;
    let refresh = resp
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expires_in = resp
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let email = resp
        .pointer("/user/email")
        .and_then(|v| v.as_str())
        .unwrap_or(email_fallback)
        .to_string();
    let now = now_unix() as i64;
    let session = SessionFile {
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        email: email.clone(),
        expires_at: now + expires_in,
    };
    save_session(&session)?;
    if let Ok(mut g) = state().lock() {
        g.session = session;
        g.status.signed_in = true;
        g.status.email = email;
        g.status.last_error = None;
    }
    Ok(())
}

fn auth_err_message(resp: &serde_json::Value) -> String {
    resp.get("error_description")
        .or_else(|| resp.get("msg"))
        .or_else(|| resp.get("message"))
        .or_else(|| resp.get("error"))
        .and_then(|v| v.as_str())
        .unwrap_or("Sign-in failed")
        .to_string()
}

fn with_cfg<T>(f: impl FnOnce(&CloudConfig) -> T) -> Result<T, String> {
    let g = state().lock().map_err(|_| "cloud lock".to_string())?;
    let cfg = g.cfg.as_ref().ok_or_else(|| "Cloud not configured".to_string())?;
    Ok(f(cfg))
}

fn set_error(msg: impl Into<String>) {
    if let Ok(mut g) = state().lock() {
        g.status.last_error = Some(msg.into());
        g.status.syncing = false;
    }
}

fn set_sync_ok() {
    if let Ok(mut g) = state().lock() {
        g.status.last_sync = Some(now_unix());
        g.status.last_error = None;
        g.status.syncing = false;
        g.last_heartbeat = Some(Instant::now());
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ensure_access_token() -> Result<(String, String, String), String> {
    let (url, key, mut session) = {
        let g = state().lock().map_err(|_| "cloud lock".to_string())?;
        let cfg = g.cfg.as_ref().ok_or_else(|| "Cloud not configured".to_string())?;
        if g.session.access_token.is_empty() {
            return Err("Not signed in".into());
        }
        (
            cfg.url.clone(),
            cfg.publishable_key.clone(),
            g.session.clone(),
        )
    };

    let now = now_unix() as i64;
    if session.expires_at > 0 && now >= session.expires_at.saturating_sub(60) {
        if session.refresh_token.is_empty() {
            return Err("Session expired — sign in again".into());
        }
        let body = serde_json::json!({
            "refresh_token": session.refresh_token,
        });
        let resp = http_json(
            "POST",
            &format!("{url}/auth/v1/token?grant_type=refresh_token"),
            &key,
            None,
            Some(&body),
        )?;
        apply_token_response(&resp, &session.email)?;
        session = state()
            .lock()
            .map_err(|_| "cloud lock".to_string())?
            .session
            .clone();
    }
    Ok((url, key, session.access_token))
}

fn http_json(
    method: &str,
    url: &str,
    apikey: &str,
    bearer: Option<&str>,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut req = ureq::request(method, url)
        .set("apikey", apikey)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    if let Some(token) = bearer {
        req = req.set("Authorization", &format!("Bearer {token}"));
    } else {
        req = req.set("Authorization", &format!("Bearer {apikey}"));
    }
    let result = match body {
        Some(b) => req.send_json(b.clone()),
        None => req.call(),
    };
    match result {
        Ok(resp) => {
            let text = resp.into_string().unwrap_or_default();
            if text.trim().is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                serde_json::from_str(&text).map_err(|e| format!("json: {e}"))
            }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .map(|v| auth_err_message(&v))
                .filter(|s| !s.is_empty())
                .unwrap_or(text);
            Err(format!("HTTP {code}: {msg}"))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn rest_upsert(
    table: &str,
    rows: &serde_json::Value,
    on_conflict: &str,
) -> Result<(), String> {
    let (url, key, token) = ensure_access_token()?;
    let endpoint = format!("{url}/rest/v1/{table}?on_conflict={on_conflict}");
    let req = ureq::request("POST", &endpoint)
        .set("apikey", &key)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("Prefer", "resolution=merge-duplicates,return=minimal");
    match req.send_json(rows.clone()) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Err(format!("upsert {table} HTTP {code}: {text}"))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn rest_insert(table: &str, row: &serde_json::Value) -> Result<(), String> {
    let (url, key, token) = ensure_access_token()?;
    let endpoint = format!("{url}/rest/v1/{table}");
    let req = ureq::request("POST", &endpoint)
        .set("apikey", &key)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("Prefer", "return=minimal");
    match req.send_json(row.clone()) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Err(format!("insert {table} HTTP {code}: {text}"))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Upsert device + senders from local config. Safe to call when offline / unsigned-in.
pub fn sync_now(app: &Config) {
    let signed = status().signed_in && status().configured;
    if !signed {
        return;
    }
    if let Ok(mut g) = state().lock() {
        g.status.syncing = true;
    }
    let display_name = with_cfg(|c| c.display_name.clone()).unwrap_or_else(|_| "MessengerBird".into());
    let lan = primary_ipv4();
    let ts = tailscale_ipv4();
    let magic = hostname();
    let auth_on = !app.auth_token.trim().is_empty();
    let now_iso = chrono_like_now();

    let device_key = app.device_id.clone();
    let owner = jwt_sub(
        &state()
            .lock()
            .ok()
            .map(|g| g.session.access_token.clone())
            .unwrap_or_default(),
    );
    let device = serde_json::json!([{
        "device_key": device_key.clone(),
        "owner_id": owner,
        "display_name": display_name,
        "lan_ip": lan,
        "tailscale_ip": ts,
        "magicdns": magic,
        "auth_token_enabled": auth_on,
        "updated_at": now_iso,
        "last_seen_at": now_iso,
    }]);

    let senders: Vec<serde_json::Value> = app
        .senders
        .iter()
        .map(|s| sender_row(s, &now_iso, &device_key))
        .collect();
    let senders_json = serde_json::Value::Array(senders);

    match (|| -> Result<(), String> {
        rest_upsert("device", &device, "device_key")?;
        if !senders_json.as_array().map(|a| a.is_empty()).unwrap_or(true) {
            rest_upsert("senders", &senders_json, "device_key,id")?;
        }
        flush_outbox()?;
        Ok(())
    })() {
        Ok(()) => set_sync_ok(),
        Err(e) => set_error(e),
    }
}

fn sender_row(s: &SenderCfg, now_iso: &str, device_key: &str) -> serde_json::Value {
    serde_json::json!({
        "device_key": device_key,
        "id": s.id,
        "name": s.display_name(),
        "port": s.port(),
        "phrase": s.phrase(),
        "listen": s.listen,
        "muted": s.muted,
        "updated_at": now_iso,
    })
}

fn outbox_path() -> PathBuf {
    locate_beside_app("config/tx_outbox.jsonl")
}

fn enqueue_outbox(row: &serde_json::Value) {
    let path = outbox_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut line = row.to_string();
    line.push('\n');
    use std::io::Write;
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn flush_outbox() -> Result<(), String> {
    let path = outbox_path();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };
    let mut kept = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if rest_insert("tx_events", &row).is_err() {
            kept.push(line.to_string());
        }
    }
    if kept.is_empty() {
        let _ = fs::remove_file(path);
    } else {
        let _ = fs::write(path, kept.join("\n") + "\n");
    }
    Ok(())
}

fn jwt_sub(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = b64url_decode(payload)?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("sub").and_then(|s| s.as_str()).map(|s| s.to_string())
}

fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut s = input.replace('-', "+").replace('_', "/");
    while !s.len().is_multiple_of(4) {
        s.push('=');
    }
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let n = |c: u8| T.iter().position(|x| *x == c).unwrap_or(0) as u32;
        let chunk = (n(bytes[i]) << 18) | (n(bytes[i + 1]) << 12) | (n(bytes[i + 2]) << 6) | n(bytes[i + 3]);
        out.push((chunk >> 16) as u8);
        if bytes[i + 2] != b'=' {
            out.push((chunk >> 8) as u8);
        }
        if bytes[i + 3] != b'=' {
            out.push(chunk as u8);
        }
        i += 4;
    }
    Some(out)
}

fn chrono_like_now() -> String {
    // RFC3339-ish UTC from unix seconds (good enough for PostgREST timestamptz).
    let secs = now_unix();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let s = rem % 60;
    // 1970-01-01 + days — use a simple civil date approximation via known epoch math.
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{mins:02}:{s:02}Z")
}

/// Algorithm from Howard Hinnant (public domain) — days since 1970-01-01 → Y-M-D.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Heartbeat: refresh last_seen_at on an interval when signed in.
pub fn tick_heartbeat(app: &Config) {
    let interval = with_cfg(|c| c.sync_interval_secs).unwrap_or(60).max(15);
    let due = {
        let g = match state().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if !g.status.configured || !g.status.signed_in {
            return;
        }
        match g.last_heartbeat {
            None => true,
            Some(t) => t.elapsed() >= Duration::from_secs(interval),
        }
    };
    if due {
        let app = app.clone();
        thread::spawn(move || sync_now(&app));
    }
}

/// Spawn background sync shortly after startup (non-blocking).
pub fn spawn_startup_sync(app: Config) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(800));
        sync_now(&app);
    });
}

#[derive(Clone, Debug)]
pub struct TxEvent {
    pub sender_id: String,
    pub phrase: String,
    pub ok: bool,
    pub source: &'static str,
    pub client_host: Option<String>,
    pub detail: Option<String>,
}

/// Fire-and-forget insert into tx_events when signed in.
pub fn log_tx(event: TxEvent) {
    let st = status();
    if !st.configured || !st.signed_in {
        return;
    }
    thread::spawn(move || {
        let device_key = crate::config::Config::load().device_id;
        let row = serde_json::json!({
            "device_key": device_key,
            "sender_id": if event.sender_id.is_empty() { serde_json::Value::Null } else { serde_json::json!(event.sender_id) },
            "phrase": event.phrase,
            "ok": event.ok,
            "source": event.source,
            "client_host": event.client_host,
            "detail": event.detail,
            "fired_at": chrono_like_now(),
        });
        if let Err(e) = rest_insert("tx_events", &row) {
            enqueue_outbox(&row);
            set_error(e);
        }
    });
}

pub fn log_tx_simple(
    sender_id: &str,
    phrase: &str,
    ok: bool,
    source: &'static str,
    client_host: Option<String>,
    detail: Option<String>,
) {
    log_tx(TxEvent {
        sender_id: sender_id.to_string(),
        phrase: phrase.to_string(),
        ok,
        source,
        client_host,
        detail,
    });
}
