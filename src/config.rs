use crate::netinfo::{force_public_bind, listen_port};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub radio: RadioCfg,
    /// One independent HTTP endpoint per announcing machine.
    #[serde(default = "default_senders")]
    pub senders: Vec<SenderCfg>,
    /// Shared token for POST /tx. Clients send `X-MessengerBird-Token` or `Authorization: Bearer …`.
    #[serde(default)]
    pub auth_token: String,
    /// When false, POST /tx is open and an empty token is not auto-filled.
    #[serde(default = "default_true")]
    pub auth_enabled: bool,
    /// Stable cloud device id (UUID). Empty until first save.
    #[serde(default)]
    pub device_id: String,
    /// Max POST /tx per client IP per minute. 0 = unlimited.
    #[serde(default = "default_rate")]
    pub rate_per_minute: u32,
    /// Optional CIDR allowlist for POST /tx. Empty = any source.
    #[serde(default)]
    pub allow_cidrs: Vec<String>,
    #[serde(default)]
    pub wizard_done: bool,
    #[serde(skip)]
    pub token_just_minted: bool,
    #[serde(skip)]
    pub needs_save: bool,
    /// Legacy single listen address (migrated into `senders` on load).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SenderCfg {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub listen: String,
    /// Fixed phrase / WAV slug this endpoint always transmits.
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub muted: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RadioCfg {
    #[serde(default = "default_channel")]
    pub channel: u8,
    #[serde(default = "default_squelch")]
    pub squelch: u8,
    #[serde(default)]
    pub ctcss: u8,
    #[serde(default = "default_uart")]
    pub uart_port: String,
    #[serde(default = "default_ch1")]
    pub ch_bit1_gpio: u8,
    #[serde(default = "default_ch2")]
    pub ch_bit2_gpio: u8,
    #[serde(default = "default_ch4")]
    pub ch_bit4_gpio: u8,
    #[serde(default = "default_ch8")]
    pub ch_bit8_gpio: u8,
    #[serde(default = "default_ptt")]
    pub ptt_gpio: u8,
    #[serde(default = "default_spken")]
    pub spken_gpio: u8,
    #[serde(default = "default_audio")]
    pub audio_device: String,
    #[serde(default)]
    pub muted: bool,
    /// Seconds to wait for SPKEN low before the force-TX decision.
    #[serde(default = "default_wait")]
    pub channel_wait_secs: u64,
    /// When true (SiloMonitor default), transmit after the wait even if still busy.
    #[serde(default = "default_true")]
    pub force_tx_if_busy: bool,
    /// Skip GPIO and aplay (desktop development without a Pi).
    #[serde(default)]
    pub dry_run: bool,
}

fn default_channel() -> u8 {
    3
}
fn default_squelch() -> u8 {
    1
}
fn default_uart() -> String {
    "/dev/ttyAMA0".into()
}
fn default_ch1() -> u8 {
    5
}
fn default_ch2() -> u8 {
    6
}
fn default_ch4() -> u8 {
    12
}
fn default_ch8() -> u8 {
    13
}
fn default_ptt() -> u8 {
    23
}
fn default_spken() -> u8 {
    24
}
fn default_audio() -> String {
    "plughw:CARD=Headphones,DEV=0".into()
}
fn default_true() -> bool {
    true
}
fn default_wait() -> u64 {
    30
}
fn default_rate() -> u32 {
    30
}

fn default_senders() -> Vec<SenderCfg> {
    vec![
        SenderCfg {
            id: "silo_empty".into(),
            name: "Silo is empty".into(),
            listen: "0.0.0.0:8787".into(),
            message: "Silo is empty".into(),
            muted: false,
        },
        SenderCfg {
            id: "check_camera".into(),
            name: "Check camera".into(),
            listen: "0.0.0.0:8788".into(),
            message: "Check camera".into(),
            muted: false,
        },
        SenderCfg {
            id: "starlinger_red".into(),
            name: "Starlinger red".into(),
            listen: "0.0.0.0:8789".into(),
            message: "Starlinger red".into(),
            muted: false,
        },
    ]
}

impl Default for RadioCfg {
    fn default() -> Self {
        Self {
            channel: default_channel(),
            squelch: default_squelch(),
            ctcss: 0,
            uart_port: default_uart(),
            ch_bit1_gpio: default_ch1(),
            ch_bit2_gpio: default_ch2(),
            ch_bit4_gpio: default_ch4(),
            ch_bit8_gpio: default_ch8(),
            ptt_gpio: default_ptt(),
            spken_gpio: default_spken(),
            audio_device: default_audio(),
            muted: false,
            channel_wait_secs: default_wait(),
            force_tx_if_busy: true,
            dry_run: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            radio: RadioCfg::default(),
            senders: default_senders(),
            auth_token: String::new(),
            auth_enabled: true,
            device_id: String::new(),
            rate_per_minute: default_rate(),
            allow_cidrs: Vec::new(),
            wizard_done: false,
            token_just_minted: false,
            needs_save: false,
            listen: None,
            source_path: None,
        }
    }
}

impl SenderCfg {
    pub fn display_name(&self) -> &str {
        if !self.name.trim().is_empty() {
            self.name.trim()
        } else if !self.message.trim().is_empty() {
            self.message.trim()
        } else {
            self.id.as_str()
        }
    }

    pub fn phrase(&self) -> &str {
        if !self.message.trim().is_empty() {
            self.message.trim()
        } else {
            self.display_name()
        }
    }

    pub fn port(&self) -> u16 {
        listen_port(&self.listen)
    }

    /// URL VisionLab hits for this sound (`host` = Tailscale/LAN/public, no port).
    pub fn remote_tx_url(&self, host: &str) -> String {
        format!("http://{}:{}/tx", host.trim(), self.port())
    }
}

impl Config {
    pub fn path() -> PathBuf {
        if let Ok(cwd) = std::env::current_dir() {
            let p = cwd.join("config.yaml");
            if p.exists() {
                return p;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let p = dir.join("config.yaml");
                if p.exists() {
                    return p;
                }
                if let Some(root) = exe.ancestors().nth(3) {
                    let p = root.join("config.yaml");
                    if p.exists() {
                        return p;
                    }
                }
            }
        }
        PathBuf::from("config.yaml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Self {
        let mut cfg = if path.exists() {
            match fs::read_to_string(path) {
                Ok(text) => serde_yaml::from_str(&text).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        };
        cfg.normalize();
        cfg.source_path = Some(path.to_path_buf());
        cfg
    }

    fn normalize(&mut self) {
        self.radio.channel = crate::peltor::clamp_channel(self.radio.channel);
        self.radio.ctcss = crate::peltor::clamp_ctcss(self.radio.ctcss);
        self.radio.squelch = crate::sa828::clamp_squelch(self.radio.squelch);

        if self.senders.is_empty() {
            if let Some(listen) = self.listen.clone().filter(|s| !s.trim().is_empty()) {
                self.senders = vec![SenderCfg {
                    id: "default".into(),
                    name: "Alert".into(),
                    listen,
                    message: "Alert".into(),
                    muted: false,
                }];
            } else {
                self.senders = default_senders();
            }
        }

        for (i, sender) in self.senders.iter_mut().enumerate() {
            if sender.id.trim().is_empty() {
                sender.id = format!("sender_{}", i + 1);
            }
            if sender.listen.trim().is_empty() {
                sender.listen = format!("0.0.0.0:{}", 8787 + i as u16);
            }
            // Always bind all interfaces so Tailscale / WAN / another PC can reach us.
            sender.listen = force_public_bind(&sender.listen);
            if sender.message.trim().is_empty() {
                sender.message = if sender.name.trim().is_empty() {
                    sender.id.clone()
                } else {
                    sender.name.clone()
                };
            }
            if sender.name.trim().is_empty() {
                sender.name = sender.message.clone();
            }
        }

        self.auth_token = self.auth_token.trim().to_string();
        if self.radio.channel_wait_secs == 0 {
            self.radio.channel_wait_secs = default_wait();
        }
        if self.device_id.trim().is_empty() {
            self.device_id = new_uuid();
            self.needs_save = true;
        }
        if self.auth_enabled && self.auth_token.is_empty() {
            self.auth_token = random_token();
            self.token_just_minted = true;
            self.needs_save = true;
        }

        // Drop legacy key so saves stay on the multi-sender shape.
        self.listen = None;
    }

    /// Persist a freshly minted token / device id after load.
    pub fn ensure_persisted_secrets(&mut self) {
        if self.needs_save {
            let _ = self.save();
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self.source_path.clone().unwrap_or_else(Self::path);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let text = serde_yaml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(&path, text).map_err(|e| e.to_string())
    }
}

fn random_bytes(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    } else {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((t >> ((i % 8) * 8)) as u8).wrapping_add(i as u8);
        }
    }
    buf
}

pub fn random_token() -> String {
    random_bytes(32)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn new_uuid() -> String {
    let mut b = random_bytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    )
}
