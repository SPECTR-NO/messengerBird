#!/usr/bin/env bash
# MessengerBird — build + launch the desktop radio sender (SiloMonitor-style).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

unset CARGO_TARGET_DIR

BIN="$ROOT/target/release/messenger-bird"
if [[ ! -x "$BIN" ]] || find src Cargo.toml -newer "$BIN" 2>/dev/null | grep -q .; then
  echo "Building MessengerBird…"
  cargo build --release
fi

export DISPLAY="${DISPLAY:-:0}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

exec "$BIN" "$@"
