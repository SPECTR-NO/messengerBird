//! Pick a host string VisionLab can paste (Tailscale preferred, else LAN).

use std::net::UdpSocket;
use std::process::Command;

/// Best host for remote clients: Tailscale IPv4 → primary LAN IPv4 → hostname.
pub fn visionlab_host() -> String {
    if let Some(ip) = tailscale_ipv4() {
        return ip;
    }
    if let Some(ip) = primary_ipv4() {
        return ip;
    }
    hostname().unwrap_or_else(|| "127.0.0.1".into())
}

pub fn host_source_label() -> &'static str {
    if tailscale_ipv4().is_some() {
        "Tailscale"
    } else if primary_ipv4().is_some() {
        "LAN"
    } else {
        "local"
    }
}

pub fn tailscale_ipv4() -> Option<String> {
    let output = Command::new("tailscale").args(["ip", "-4"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let ip = line.trim();
        if is_tailscale_ip(ip) {
            return Some(ip.to_string());
        }
    }
    None
}

fn is_tailscale_ip(ip: &str) -> bool {
    // Tailscale CGNAT: 100.64.0.0/10
    let parts: Vec<_> = ip.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    let Ok(a) = parts[0].parse::<u8>() else {
        return false;
    };
    let Ok(b) = parts[1].parse::<u8>() else {
        return false;
    };
    a == 100 && (64..=127).contains(&b)
}

pub fn primary_ipv4() -> Option<String> {
    // UDP connect trick — no packets sent; reveals the outbound interface IP.
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("1.1.1.1:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() {
        return None;
    }
    Some(ip.to_string())
}

pub fn hostname() -> Option<String> {
    let output = Command::new("hostname").arg("-s").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Rewrite any listen string so the bind host is always `0.0.0.0` (keep port).
pub fn force_public_bind(listen: &str) -> String {
    let listen = listen.trim();
    if listen.is_empty() {
        return "0.0.0.0:8787".into();
    }
    if let Some(port) = listen.strip_prefix(':') {
        return format!("0.0.0.0:{port}");
    }
    if listen.chars().all(|c| c.is_ascii_digit()) {
        return format!("0.0.0.0:{listen}");
    }
    if let Some((host, port)) = listen.rsplit_once(':') {
        let host = host.trim();
        if host.is_empty()
            || host.eq_ignore_ascii_case("localhost")
            || host == "127.0.0.1"
            || host == "::1"
            || host == "[::1]"
        {
            return format!("0.0.0.0:{port}");
        }
        // Keep intentional non-local binds (rare); still prefer all-interfaces.
        if host == "0.0.0.0" || host == "*" || host == "[::]" {
            return format!("0.0.0.0:{port}");
        }
        // VisionLab remote path needs all interfaces — force 0.0.0.0.
        return format!("0.0.0.0:{port}");
    }
    format!("0.0.0.0:{listen}")
}

pub fn listen_port(listen: &str) -> u16 {
    listen
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .unwrap_or(8787)
}
