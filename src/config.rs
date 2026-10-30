use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub radio: RadioCfg,
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

fn default_listen() -> String {
    "0.0.0.0:8787".into()
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
}

fn default_channel() -> u8 {
    1
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
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            radio: RadioCfg::default(),
            listen: default_listen(),
            source_path: None,
        }
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
                // cargo run: target/release → repo root
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
        cfg.radio.channel = crate::peltor::clamp_channel(cfg.radio.channel);
        cfg.radio.ctcss = crate::peltor::clamp_ctcss(cfg.radio.ctcss);
        cfg.radio.squelch = crate::sa828::clamp_squelch(cfg.radio.squelch);
        if cfg.listen.trim().is_empty() {
            cfg.listen = default_listen();
        }
        cfg.source_path = Some(path.to_path_buf());
        cfg
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self
            .source_path
            .clone()
            .unwrap_or_else(Self::path);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let text = serde_yaml::to_string(self).map_err(|e| e.to_string())?;
        fs::write(&path, text).map_err(|e| e.to_string())
    }
}
