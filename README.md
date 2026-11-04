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

Prefer **Tailscale**. Do not publish 8787–8789 on the public internet without the auth token. This app speaks plain HTTP; terminate HTTPS on a reverse proxy if you need it.

## Install (boot)

```bash
sudo ./install.sh
systemctl start messengerbird
```

Installs to `/opt/messengerbird`, enables `messengerbird.service`, and writes a desktop entry that does not hardcode a home directory. `./start.sh` remains the dev build-and-run path.

## RF / plant use

MessengerBird keys an analog PMR446 SA828 on the channel selected in the app (default channel 3) using the same open-drain PTT sequence as SiloMonitor. Use it only on a channel and tone your plant is licensed or permitted to use. Keep the module idle (PTT High-Z) when not announcing.

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

### Auth (on by default)

The first launch writes a random `auth_token` into `config.yaml` when `auth_enabled: true`. Copy it into VisionLab.

`POST /tx` requires:

```http
X-MessengerBird-Token: <token>
# or
Authorization: Bearer <token>
```

`GET /health` stays open so VisionLab can probe reachability without a token.

Set `auth_enabled: false` only in a locked-down lab. The app shows a red warning while TX is open.

Busy contract: a free radio returns **200** `{ ok, queued: false }`. If a clip is already playing, the next POST is **202** `{ queued: true }` (FIFO, depth 8). A full queue returns **503** with `Retry-After` — retry only then.

### Smoke test

```bash
TOKEN=… ./scripts/smoke.sh
```

On the Pi, also: UI **Read**, confirm channel 3, **TX** each sender and hear it, toggle mute and restart (mute stays), PTT idle after each clip.

## MessengerBird cloud (Supabase)

Separate from VisionLab’s database. Device identity, senders, and a Pi-side `tx_events` log.

1. Apply SQL: `supabase/migrations/20261103180000_messengerbird_cloud.sql` in the project SQL editor.
2. Copy `config/supabase.example.yaml` → `config/supabase.yaml` (gitignored) with URL + publishable key.
3. Apply `supabase/migrations/20261104120000_multi_device.sql` as well (device id + owner RLS).
4. In the app **Cloud** panel: create/sign in with email+password. Confirm the email if asked, or use **Reset** to send a recovery link. Sign out revokes the refresh token. Offline radio + HTTP keep working without sign-in. Failed `tx_events` inserts retry from `config/tx_outbox.jsonl`.

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
