#!/usr/bin/env bash
# Install MessengerBird under /opt/messengerbird and enable a boot service.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-/opt/messengerbird}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  echo "Re-run as root: sudo $0"
  exit 1
fi

echo "Building MessengerBird ${VERSION}…"
sudo -u "${SUDO_USER:-$USER}" bash -lc "cd '$ROOT' && cargo build --release"

install -d "$PREFIX/audio" "$PREFIX/assets" "$PREFIX/config" /usr/local/bin
install -m 0755 "$ROOT/target/release/messenger-bird" "$PREFIX/messenger-bird"
install -m 0755 "$ROOT/start.sh" "$PREFIX/start.sh"
cp -a "$ROOT/audio/." "$PREFIX/audio/"
install -m 0644 "$ROOT/assets/icon.png" "$PREFIX/assets/icon.png"
if [[ ! -f "$PREFIX/config.yaml" ]]; then
  install -m 0644 "$ROOT/config.yaml" "$PREFIX/config.yaml"
fi
if [[ -f "$ROOT/config/supabase.example.yaml" && ! -f "$PREFIX/config/supabase.example.yaml" ]]; then
  install -m 0644 "$ROOT/config/supabase.example.yaml" "$PREFIX/config/supabase.example.yaml"
fi
ln -sfn "$PREFIX/messenger-bird" /usr/local/bin/messenger-bird

RUN_USER="${SUDO_USER:-pi}"
cat > /etc/systemd/system/messengerbird.service <<EOF
[Unit]
Description=MessengerBird SA828 radio sender
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=${RUN_USER}
WorkingDirectory=${PREFIX}
Environment=DISPLAY=:0
ExecStart=${PREFIX}/messenger-bird
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/logrotate.d/messengerbird <<EOF
/var/log/messengerbird.log {
  weekly
  rotate 4
  compress
  missingok
  notifempty
  copytruncate
}
EOF

DESKTOP_DIR="/usr/share/applications"
install -d "$DESKTOP_DIR"
cat > "$DESKTOP_DIR/messengerbird.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=MessengerBird
Comment=Plant radio sender · SA828
Exec=${PREFIX}/messenger-bird
Path=${PREFIX}
Icon=${PREFIX}/assets/icon.png
Terminal=false
Categories=Utility;
StartupWMClass=messenger-bird
EOF

systemctl daemon-reload
systemctl enable messengerbird.service
echo "Installed ${VERSION} to ${PREFIX}. Start with: systemctl start messengerbird"
