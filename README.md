# MessengerBird

Desktop SA828 radio sender for **Raspberry Pi 5** — same radio stack as [SiloMonitor](https://github.com/makwansoran/SiloMonitor) (`rppal` UART + GPIO, eframe UI), without vision.

Each announcement is its **own sender** (own port + config). All senders share one radio channel. Different machines POST their own URL.

## Run

```bash
# Pi 5 UART once: /boot/firmware/config.txt → dtoverlay=uart0-pi5  (then reboot)
chmod +x start.sh
./start.sh
```

## Senders (default)

| Machine announcement | Port | POST |
|----------------------|------|------|
| Silo is empty | `8787` | `http://<pi-ip>:8787/tx` |
| Check camera | `8788` | `http://<pi-ip>:8788/tx` |
| Starlinger red | `8789` | `http://<pi-ip>:8789/tx` |

Body is optional — each port always plays its fixed message. Shared channel is **3** in `config.yaml`.

## Audio

| Message | WAV |
|---------|-----|
| `Silo is empty` | `audio/silo_is_empty_vox.wav` |
| `Check camera` | `audio/check_camera_vox.wav` |
| `Starlinger red` | `audio/starlinger_red_vox.wav` |

Fallback: `<slug>.wav` / `<slug>_vox.wav`, else `alert.wav`.

## Hardware (same as SiloMonitor)

| SA828 | Pi |
|-------|-----|
| RXD / TXD | pins 8 / 10 · `/dev/ttyAMA0` · 9600 |
| PTT | BCM 23 · open-drain LOW = TX |
| SPKEN | BCM 24 |
| ch 4/5/7/6 | BCM 5/6/12/13 · **active-low** |
| CS | 3.3V · VCC 5V · common GND |
