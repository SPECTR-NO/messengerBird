#!/usr/bin/env bash
# Smoke checks for a running MessengerBird. Audio on channel 3 must be confirmed by ear.
set -euo pipefail
HOST="${HOST:-127.0.0.1}"
TOKEN="${TOKEN:-}"
PORTS="${PORTS:-8787 8788 8789}"

auth=()
if [[ -n "$TOKEN" ]]; then
  auth=(-H "X-MessengerBird-Token: $TOKEN")
fi

echo "Health"
for p in $PORTS; do
  curl -fsS "http://${HOST}:${p}/health"
  echo
done

echo "TX (queue contract: 200 started, 202 queued, 503+Retry-After if full)"
for p in $PORTS; do
  code="$(curl -sS -o /tmp/mb-tx.json -w '%{http_code}' -X POST "${auth[@]}" "http://${HOST}:${p}/tx" || true)"
  echo "port ${p} -> ${code} $(cat /tmp/mb-tx.json)"
done

if [[ ! -e /dev/gpiomem && ! -e /dev/gpiochip0 ]]; then
  echo "No GPIO — this host is not the plant Pi. Confirm PTT idle + ch3 audio on the Pi by ear."
  exit 0
fi

echo "GPIO present. Confirm in the UI: Read module, channel 3 selected, each phrase heard once, PTT idle after."
