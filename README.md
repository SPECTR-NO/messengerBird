# MessengerBird

Desktop SA828 radio sender for **Raspberry Pi 5** — same radio stack as [SiloMonitor](https://github.com/makwansoran/SiloMonitor) (`rppal` UART + GPIO, eframe UI), without vision.

VisionLab (Jetson) detects → `POST /tx` → this app keys PTT and plays a WAV on UHF.

## Run

```bash
# Pi 5 UART once: /boot/firmware/config.txt → dtoverlay=uart0-pi5  (then reboot)
chmod +x start.sh
./start.sh
```

Opens a small desktop window. Also listens on **`:8787`** for VisionLab.

## VisionLab

Settings → Radio → **Radio URL**:

```
http://<pi-ip>:8787/tx
```

## Audio

Put clips in `audio/`:

- `alert.wav` — default
- `<slug>.wav` — matches announcement name (`Silo empty` → `silo_empty.wav`)

## Hardware (same as SiloMonitor)

| SA828 | Pi |
|-------|-----|
| RXD / TXD | pins 8 / 10 · `/dev/ttyAMA0` · 9600 |
| PTT | BCM 23 · open-drain LOW = TX |
| SPKEN | BCM 24 |
| ch 4/5/7/6 | BCM 5/6/12/13 · **active-low** |
| CS | 3.3V · VCC 5V · common GND |
