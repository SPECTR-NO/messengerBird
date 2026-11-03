# MessengerBird

Desktop SA828 radio sender for **Raspberry Pi 5** — same radio stack as [SiloMonitor](https://github.com/makwansoran/SiloMonitor) (`rppal` UART + GPIO, eframe UI), without vision.

Each announcement is its **own sender** (own port + fixed phrase). All senders share one radio channel. VisionLab (or any client) POSTs that sender’s URL to transmit.

## Run

```bash
# Pi 5 UART once: /boot/firmware/config.txt → dtoverlay=uart0-pi5  (then reboot)
chmod +x start.sh
./start.sh
```

Listeners bind **`0.0.0.0:<port>`** so another PC on LAN, Tailscale, or a forwarded WAN path can reach the Pi.

## VisionLab remote test

VisionLab host field = **this Pi’s address only (no port)**:

1. Prefer **Tailscale** IP / MagicDNS (shown in the app under **Remote test**).
2. Else LAN IP while on the same network.
3. Else public IP + **port-forward TCP 8787–8789** to the Pi.

| Sound | Port | Probe | Transmit |
|-------|------|-------|----------|
| Silo is empty | `8787` | `GET http://<host>:8787/health` → `{ ok, id }` | `POST http://<host>:8787/tx` |
| Check camera | `8788` | `GET …:8788/health` | `POST …:8788/tx` |
| Starlinger red | `8789` | `GET …:8789/health` | `POST …:8789/tx` |

- Body optional — each port always plays its fixed message.
- Also accepted: `POST /api/tx`, `POST /webhook`.
- `GET /` returns status JSON for that sender.

```bash
curl -s http://<host>:8787/health
curl -s -X POST http://<host>:8787/tx
```

### Auth (optional, off by default)

In `config.yaml`:

```yaml
auth_token: ""   # empty = off; VisionLab needs no header
```

When set, `POST /tx` requires:

```http
X-MessengerBird-Token: <token>
# or
Authorization: Bearer <token>
```

`GET /health` stays open so VisionLab can probe reachability without a token.

## MessengerBird cloud (Supabase)

Separate from VisionLab’s database. Device identity, senders, and a Pi-side `tx_events` log.

1. Apply SQL: `supabase/migrations/20261103180000_messengerbird_cloud.sql` in the project SQL editor.
2. Copy `config/supabase.example.yaml` → `config/supabase.yaml` (gitignored) with URL + publishable key.
3. In the app **Cloud** panel: create/sign in with email+password (authenticated RLS). Offline radio + HTTP keep working without sign-in.

## Audio

| Message | WAV |
|---------|-----|
| `Silo is empty` | `audio/silo_is_empty.wav` (or `_vox`) |
| `Check camera` | `audio/check_camera.wav` |
| `Starlinger red` | `audio/starlinger_red.wav` |

Fallback: `<slug>.wav` / `<slug>_vox.wav`, else `alert.wav`.

## Hardware (same as SiloMonitor)

| SA828 | Pi |
|-------|-----|
| RXD / TXD | pins 8 / 10 · `/dev/ttyAMA0` · 9600 |
| PTT | BCM 23 · open-drain LOW = TX |
| SPKEN | BCM 24 |
| ch 4/5/7/6 | BCM 5/6/12/13 · **active-low** |
| CS | 3.3V · VCC 5V · common GND |
