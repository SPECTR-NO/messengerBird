//! SA828 TX path — ported from SiloMonitor `silo_alert.rs`.
//!
//! Wiring (Pi BCM):
//!   PTT (SA828 pin 20) → GPIO23 — open-drain: OUTPUT LOW = TX; idle = INPUT High-Z
//!   SPKEN (SA828 pin 9) → GPIO24 — high = busy
//!   Channel bits 4/5/7/6 → GPIO 5/6/12/13 — active-low
//!   UART 9600 → /dev/ttyAMA0 (Pi 5: dtoverlay=uart0-pi5)
//!   Audio: aplay → ALSA → pot → SA828 MIC+

use serde::{Deserialize, Serialize};
use rppal::gpio::{Gpio, OutputPin};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_AUDIO: &str = "plughw:CARD=Headphones,DEV=0";
const TX_MAX: Duration = Duration::from_secs(20);
const CHANNEL_CLEAR_HOLD: Duration = Duration::from_millis(250);

static TX_BUSY: AtomicBool = AtomicBool::new(false);
static PTT_KEYED: AtomicBool = AtomicBool::new(false);
static PTT_PIN: AtomicU8 = AtomicU8::new(0);
static SPKEN_PIN: AtomicU8 = AtomicU8::new(0);
static CH_BIT1: AtomicU8 = AtomicU8::new(0);
static CH_BIT2: AtomicU8 = AtomicU8::new(0);
static CH_BIT4: AtomicU8 = AtomicU8::new(0);
static CH_BIT8: AtomicU8 = AtomicU8::new(0);
static MUTED: AtomicBool = AtomicBool::new(false);
static DRY_RUN: AtomicBool = AtomicBool::new(false);
static FORCE_TX_IF_BUSY: AtomicBool = AtomicBool::new(true);
static CHANNEL_WAIT_SECS: AtomicU64 = AtomicU64::new(30);
static AUDIO_DEVICE: Mutex<String> = Mutex::new(String::new());
static STATUS: Mutex<Option<String>> = Mutex::new(None);
static LAST_MESSAGE: Mutex<Option<String>> = Mutex::new(None);
static HISTORY: Mutex<VecDeque<AlertEvent>> = Mutex::new(VecDeque::new());
const HISTORY_MAX: usize = 200;
const QUEUE_MAX: usize = 8;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AlertEvent {
    /// Unix seconds (local display formatted in UI).
    pub at: u64,
    pub message: String,
    pub ok: bool,
    pub detail: String,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub client: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxAccept {
    /// Worker started this clip immediately.
    Started,
    /// Radio was busy; clip is waiting in the FIFO.
    Queued,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn push_alert(message: impl Into<String>, ok: bool, detail: impl Into<String>) {
    push_alert_full(message, ok, detail, "", "", "");
}

pub fn push_alert_full(
    message: impl Into<String>,
    ok: bool,
    detail: impl Into<String>,
    sender_id: impl Into<String>,
    source: impl Into<String>,
    client: impl Into<String>,
) {
    let event = AlertEvent {
        at: now_unix(),
        message: message.into(),
        ok,
        detail: detail.into(),
        sender_id: sender_id.into(),
        source: source.into(),
        client: client.into(),
    };
    if let Ok(mut hist) = HISTORY.lock() {
        hist.push_front(event);
        while hist.len() > HISTORY_MAX {
            hist.pop_back();
        }
        persist_history(&hist);
    }
}

pub fn history() -> Vec<AlertEvent> {
    HISTORY
        .lock()
        .ok()
        .map(|h| h.iter().cloned().collect())
        .unwrap_or_default()
}

pub fn clear_history() {
    if let Ok(mut hist) = HISTORY.lock() {
        hist.clear();
        persist_history(&hist);
    }
}

fn history_path() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("config.yaml").exists() || cwd.join("Cargo.toml").exists() {
            return cwd.join("config").join("history.jsonl");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("config").join("history.jsonl");
        }
    }
    PathBuf::from("config/history.jsonl")
}

fn persist_history(hist: &VecDeque<AlertEvent>) {
    let path = history_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut text = String::new();
    for event in hist.iter().rev().take(HISTORY_MAX) {
        if let Ok(line) = serde_json::to_string(event) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    let _ = std::fs::write(path, text);
}

/// Load persisted alerts. Newest stays at the front.
pub fn load_history() {
    let path = history_path();
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let mut loaded = VecDeque::new();
    for line in text.lines() {
        if let Ok(event) = serde_json::from_str::<AlertEvent>(line) {
            loaded.push_back(event);
        }
    }
    while loaded.len() > HISTORY_MAX {
        loaded.pop_front();
    }
    // File is oldest-first; UI wants newest first.
    let newest_first: VecDeque<AlertEvent> = loaded.into_iter().rev().collect();
    if let Ok(mut hist) = HISTORY.lock() {
        *hist = newest_first;
    }
}

pub fn set_ptt_pin(pin: u8) {
    PTT_PIN.store(pin, Ordering::SeqCst);
    ensure_ptt_idle();
}

pub fn set_spken_pin(pin: u8) {
    SPKEN_PIN.store(pin, Ordering::SeqCst);
}

pub fn set_channel_pins(bit1: u8, bit2: u8, bit4: u8, bit8: u8) {
    let prev1 = CH_BIT1.swap(bit1, Ordering::SeqCst);
    let prev2 = CH_BIT2.swap(bit2, Ordering::SeqCst);
    let prev4 = CH_BIT4.swap(bit4, Ordering::SeqCst);
    let prev8 = CH_BIT8.swap(bit8, Ordering::SeqCst);
    if prev1 != bit1 || prev2 != bit2 || prev4 != bit4 || prev8 != bit8 {
        if let Ok(mut held) = CHANNEL_OUT.lock() {
            *held = None;
        }
    }
}

pub fn set_audio_device(dev: &str) {
    if let Ok(mut g) = AUDIO_DEVICE.lock() {
        *g = dev.trim().to_string();
    }
}

pub fn set_dry_run(on: bool) {
    DRY_RUN.store(on, Ordering::SeqCst);
}

pub fn set_csma(wait_secs: u64, force_tx_if_busy: bool) {
    CHANNEL_WAIT_SECS.store(wait_secs.max(1), Ordering::SeqCst);
    FORCE_TX_IF_BUSY.store(force_tx_if_busy, Ordering::SeqCst);
}

pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::SeqCst);
}

pub fn muted() -> bool {
    MUTED.load(Ordering::SeqCst)
}

pub fn busy() -> bool {
    TX_BUSY.load(Ordering::SeqCst)
}

pub fn take_status() -> Option<String> {
    STATUS.lock().ok().and_then(|mut s| s.take())
}

pub fn last_message() -> Option<String> {
    LAST_MESSAGE.lock().ok().and_then(|g| g.clone())
}

pub fn status_ok(msg: &str) -> bool {
    msg.starts_with("Played")
}

fn set_status(msg: String) {
    if let Ok(mut s) = STATUS.lock() {
        *s = Some(msg);
    }
}

/// Drive SA828 pins 4/5/7/6 so the module transmits on `ch`.
/// NiceRF encode is active-low (same as SiloMonitor).
pub fn select_channel(ch: u8) -> Result<(), String> {
    if DRY_RUN.load(Ordering::SeqCst) {
        return Ok(());
    }
    let code = crate::peltor::sa828_channel_code(ch);
    let pins = [
        CH_BIT1.load(Ordering::SeqCst),
        CH_BIT2.load(Ordering::SeqCst),
        CH_BIT4.load(Ordering::SeqCst),
        CH_BIT8.load(Ordering::SeqCst),
    ];
    if pins.contains(&0) {
        return Err(
            "Channel pins not wired. SA828 4/5/7/6 → Pi GPIO 5/6/12/13.".into(),
        );
    }
    let mut held = CHANNEL_OUT
        .lock()
        .map_err(|_| "channel gpio busy".to_string())?;
    if held.is_none() {
        *held = Some(open_channel_pins(pins)?);
    }
    let outs = held.as_mut().ok_or("channel gpio missing")?;
    drive_active_low(&mut outs.bit1, code & 0b0001 != 0);
    drive_active_low(&mut outs.bit2, code & 0b0010 != 0);
    drive_active_low(&mut outs.bit4, code & 0b0100 != 0);
    drive_active_low(&mut outs.bit8, code & 0b1000 != 0);
    Ok(())
}

fn drive_active_low(pin: &mut OutputPin, bit_set: bool) {
    if bit_set {
        pin.set_low();
    } else {
        pin.set_high();
    }
}

struct ChannelOut {
    bit1: OutputPin,
    bit2: OutputPin,
    bit4: OutputPin,
    bit8: OutputPin,
}

fn open_channel_pins(pins: [u8; 4]) -> Result<ChannelOut, String> {
    let gpio = Gpio::new().map_err(|e| format!("GPIO: {e}"))?;
    let open = |pin: u8| -> Result<OutputPin, String> {
        Ok(gpio
            .get(pin)
            .map_err(|e| format!("channel GPIO{pin}: {e}"))?
            .into_output())
    };
    Ok(ChannelOut {
        bit1: open(pins[0])?,
        bit2: open(pins[1])?,
        bit4: open(pins[2])?,
        bit8: open(pins[3])?,
    })
}

static CHANNEL_OUT: Mutex<Option<ChannelOut>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineLamp {
    Off,
    Busy,
    Tx,
}

pub fn line_lamp() -> LineLamp {
    if PTT_KEYED.load(Ordering::SeqCst) {
        return LineLamp::Tx;
    }
    if channel_busy() == Some(true) {
        return LineLamp::Busy;
    }
    LineLamp::Off
}

pub fn channel_busy() -> Option<bool> {
    read_spken_busy().ok()
}

fn float_ptt_pin(pin: u8) {
    if pin == 0 {
        return;
    }
    let Ok(gpio) = Gpio::new() else {
        return;
    };
    let Ok(p) = gpio.get(pin) else {
        return;
    };
    let mut inp = p.into_input();
    inp.set_reset_on_drop(false);
}

pub fn ensure_ptt_idle() {
    float_ptt_pin(PTT_PIN.load(Ordering::SeqCst));
}

pub fn release_ptt() {
    ensure_ptt_idle();
}

fn key_ptt_pin() -> Result<Option<OutputPin>, String> {
    let pin = PTT_PIN.load(Ordering::SeqCst);
    if pin == 0 {
        return Ok(None);
    }
    let mut out = Gpio::new()
        .and_then(|gpio| gpio.get(pin))
        .map(|p| p.into_output_low())
        .map_err(|e| format!("PTT GPIO{pin}: {e}"))?;
    out.set_reset_on_drop(false);
    Ok(Some(out))
}

struct Ptt(Option<OutputPin>);

impl Ptt {
    fn key() -> Result<Self, String> {
        let this = Self(key_ptt_pin()?);
        if this.is_keyed() {
            PTT_KEYED.store(true, Ordering::SeqCst);
        }
        Ok(this)
    }

    fn is_keyed(&self) -> bool {
        self.0.as_ref().map(|p| p.is_set_low()).unwrap_or(true)
    }
}

impl Drop for Ptt {
    fn drop(&mut self) {
        PTT_KEYED.store(false, Ordering::SeqCst);
        drop(self.0.take());
        release_ptt();
    }
}

/// Configured ALSA device only — no card auto-fallback (same as SiloMonitor).
fn audio_device() -> String {
    AUDIO_DEVICE
        .lock()
        .ok()
        .map(|g| g.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_AUDIO.to_string())
}

fn root_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.to_path_buf());
        }
        if let Some(root) = exe.ancestors().nth(3) {
            out.push(root.to_path_buf());
        }
    }
    out
}

fn phrase_slug(message: &str) -> String {
    let mut slug = String::new();
    for c in message.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "alert".into()
    } else {
        slug
    }
}

pub fn resolve_wav(message: &str) -> Option<PathBuf> {
    let slug = phrase_slug(message);
    let names = [
        format!("{slug}.wav"),
        format!("{slug}_vox.wav"),
        "alert.wav".into(),
    ];
    for root in root_dirs() {
        let audio = root.join("audio");
        for name in &names {
            let path = audio.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Optional cloud / audit metadata for a transmit.
#[derive(Clone, Debug, Default)]
pub struct TxMeta {
    pub sender_id: String,
    pub source: &'static str,
    pub client_host: Option<String>,
}

/// Fire TX. `Started` = on air now; `Queued` = waiting behind the current clip.
#[allow(dead_code)]
pub fn transmit_message(message: &str) -> Result<TxAccept, String> {
    transmit_with(message, TxMeta::default())
}

struct TxJob {
    wav: PathBuf,
    label: String,
    meta: TxMeta,
}

static TX_QUEUE: Mutex<VecDeque<TxJob>> = Mutex::new(VecDeque::new());
static WORKER: AtomicBool = AtomicBool::new(false);

pub fn queue_len() -> usize {
    TX_QUEUE.lock().map(|q| q.len()).unwrap_or(0)
}

pub fn transmit_with(message: &str, meta: TxMeta) -> Result<TxAccept, String> {
    let label = message.trim().to_string();
    let source = if meta.source.is_empty() {
        "ui"
    } else {
        meta.source
    };
    if label.is_empty() {
        let err = "Empty message".to_string();
        push_alert("Alert", false, &err);
        crate::cloud::log_tx_simple(
            &meta.sender_id,
            "Alert",
            false,
            source,
            meta.client_host.clone(),
            Some(err.clone()),
        );
        return Err(err);
    }
    if MUTED.load(Ordering::SeqCst) {
        let err = "Muted".to_string();
        push_alert_full(&label, false, &err, &meta.sender_id, source, meta.client_host.as_deref().unwrap_or(""));
        crate::cloud::log_tx_simple(
            &meta.sender_id,
            &label,
            false,
            source,
            meta.client_host.clone(),
            Some(err.clone()),
        );
        return Err(err);
    }
    let wav = match resolve_wav(&label) {
        Some(path) => path,
        None => {
            let err = format!(
                "No WAV for “{label}”. Put {}.wav or alert.wav in audio/",
                phrase_slug(&label)
            );
            push_alert_full(
                &label,
                false,
                &err,
                &meta.sender_id,
                source,
                meta.client_host.as_deref().unwrap_or(""),
            );
            crate::cloud::log_tx_simple(
                &meta.sender_id,
                &label,
                false,
                source,
                meta.client_host.clone(),
                Some(err.clone()),
            );
            return Err(err);
        }
    };
    let job = TxJob {
        wav,
        label: label.clone(),
        meta,
    };
    let mut q = TX_QUEUE.lock().map_err(|_| "tx queue busy".to_string())?;
    if WORKER.load(Ordering::SeqCst) {
        if q.len() >= QUEUE_MAX {
            let err = "Transmit queue full".to_string();
            push_alert_full(
                &label,
                false,
                &err,
                &job.meta.sender_id,
                source,
                job.meta.client_host.as_deref().unwrap_or(""),
            );
            crate::cloud::log_tx_simple(
                &job.meta.sender_id,
                &label,
                false,
                source,
                job.meta.client_host.clone(),
                Some(err.clone()),
            );
            return Err(err);
        }
        q.push_back(job);
        return Ok(TxAccept::Queued);
    }
    WORKER.store(true, Ordering::SeqCst);
    TX_BUSY.store(true, Ordering::SeqCst);
    drop(q);
    if let Ok(mut g) = LAST_MESSAGE.lock() {
        *g = Some(label);
    }
    thread::spawn(move || worker_loop(job));
    Ok(TxAccept::Started)
}

fn worker_loop(first: TxJob) {
    let mut current = Some(first);
    loop {
        let job = if let Some(job) = current.take() {
            job
        } else {
            let mut q = match TX_QUEUE.lock() {
                Ok(q) => q,
                Err(_) => {
                    WORKER.store(false, Ordering::SeqCst);
                    TX_BUSY.store(false, Ordering::SeqCst);
                    return;
                }
            };
            match q.pop_front() {
                Some(job) => job,
                None => {
                    WORKER.store(false, Ordering::SeqCst);
                    TX_BUSY.store(false, Ordering::SeqCst);
                    return;
                }
            }
        };
        TX_BUSY.store(true, Ordering::SeqCst);
        if let Ok(mut g) = LAST_MESSAGE.lock() {
            *g = Some(job.label.clone());
        }
        let source = if job.meta.source.is_empty() {
            "ui"
        } else {
            job.meta.source
        };
        let client = job.meta.client_host.clone().unwrap_or_default();
        let (ok, msg) = match play_voice_path(&job.wav) {
            Ok(()) => (true, format!("Played {}", job.label)),
            Err(e) => {
                eprintln!("radio: {e}");
                (false, e)
            }
        };
        push_alert_full(&job.label, ok, &msg, &job.meta.sender_id, source, &client);
        crate::cloud::log_tx_simple(
            &job.meta.sender_id,
            &job.label,
            ok,
            source,
            job.meta.client_host.clone(),
            Some(msg.clone()),
        );
        set_status(msg);
    }
}

/// SiloMonitor `play_voice_path`: keyed playback, then force High-Z idle.
fn play_voice_path(wav: &PathBuf) -> Result<(), String> {
    let result = play_voice_keyed(wav);
    ensure_ptt_idle();
    result
}

fn wav_pcm_duration(path: &PathBuf) -> Result<Duration, String> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    let mut f = File::open(path).map_err(|e| format!("wav open: {e}"))?;
    let mut riff = [0u8; 12];
    f.read_exact(&mut riff)
        .map_err(|e| format!("wav header: {e}"))?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }

    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data_bytes = None;

    loop {
        let mut chunk = [0u8; 8];
        if f.read_exact(&mut chunk).is_err() {
            break;
        }
        let id = &chunk[0..4];
        let size = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        if id == b"fmt " {
            let mut fmt = vec![0u8; size as usize];
            f.read_exact(&mut fmt)
                .map_err(|e| format!("wav fmt: {e}"))?;
            if fmt.len() < 16 {
                return Err("wav fmt too short".into());
            }
            channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            bits = u16::from_le_bytes([fmt[14], fmt[15]]);
        } else if id == b"data" {
            data_bytes = Some(size);
            break;
        } else {
            f.seek(SeekFrom::Current(i64::from(size)))
                .map_err(|e| format!("wav seek: {e}"))?;
        }
        if size % 2 == 1 {
            let _ = f.seek(SeekFrom::Current(1));
        }
    }

    let data_bytes = data_bytes.ok_or_else(|| "wav missing data chunk".to_string())?;
    if sample_rate == 0 || channels == 0 || bits == 0 || !bits.is_multiple_of(8) {
        return Err("wav invalid format".into());
    }
    let bytes_per_frame = u64::from(channels) * u64::from(bits / 8);
    if bytes_per_frame == 0 {
        return Err("wav invalid frame size".into());
    }
    let frames = u64::from(data_bytes) / bytes_per_frame;
    let nanos = frames
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_div(u64::from(sample_rate)))
        .ok_or_else(|| "wav duration overflow".to_string())?;
    Ok(Duration::from_nanos(nanos))
}

fn play_voice_keyed(wav: &PathBuf) -> Result<(), String> {
    if DRY_RUN.load(Ordering::SeqCst) {
        eprintln!("radio: dry-run (no GPIO / aplay) {}", wav.display());
        thread::sleep(Duration::from_millis(200));
        return Ok(());
    }

    wait_channel_clear()?;

    let duration = wav_pcm_duration(wav)?;
    // Unkey 0.5 s before the last sample so TX dies before the clip ends.
    let release_after = duration.saturating_sub(Duration::from_millis(500));

    let tx = Ptt::key()?;
    if !tx.is_keyed() {
        return Err("PTT did not go low".into());
    }

    let device = audio_device();
    let mut child = Command::new("/usr/bin/aplay")
        .args(["-D", &device, "-q"])
        .arg(wav)
        .spawn()
        .map_err(|e| format!("aplay: {e}"))?;

    let started = Instant::now();
    let deadline = started + TX_MAX;
    let mut ptt = Some(tx);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                drop(ptt.take());
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("aplay {status}"))
                };
            }
            Ok(None) => {
                if ptt.is_some() && started.elapsed() >= release_after {
                    drop(ptt.take());
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    drop(ptt.take());
                    return Err("aplay timed out".into());
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                drop(ptt.take());
                return Err(format!("aplay wait: {e}"));
            }
        }
    }
}

fn read_spken_busy() -> Result<bool, String> {
    let pin = SPKEN_PIN.load(Ordering::SeqCst);
    if pin == 0 {
        return Err("SPKEN disabled".into());
    }
    let inp = Gpio::new()
        .and_then(|gpio| gpio.get(pin))
        .map(|p| p.into_input())
        .map_err(|e| format!("SPKEN GPIO{pin}: {e}"))?;
    Ok(inp.is_high())
}

fn wait_channel_clear() -> Result<(), String> {
    let pin = SPKEN_PIN.load(Ordering::SeqCst);
    if pin == 0 {
        return Ok(());
    }
    let wait_max = Duration::from_secs(CHANNEL_WAIT_SECS.load(Ordering::SeqCst).max(1));
    let start = Instant::now();
    let mut clear_since: Option<Instant> = None;
    loop {
        let busy = match read_spken_busy() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("radio: SPKEN read failed ({e}) — transmitting without CSMA");
                return Ok(());
            }
        };
        if busy {
            clear_since = None;
        } else {
            match clear_since {
                None => clear_since = Some(Instant::now()),
                Some(t) if t.elapsed() >= CHANNEL_CLEAR_HOLD => return Ok(()),
                Some(_) => {}
            }
        }
        if start.elapsed() >= wait_max {
            if FORCE_TX_IF_BUSY.load(Ordering::SeqCst) {
                eprintln!(
                    "radio: channel busy >{}s — transmitting anyway",
                    wait_max.as_secs()
                );
                return Ok(());
            }
            return Err(format!(
                "Channel busy >{}s — not transmitting",
                wait_max.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
