# Changelog

## 0.2.0

- SA828 voice TX follows the SiloMonitor PTT / SPKEN / aplay sequence. Playback uses the configured ALSA device only.
- HTTP auth on by default (token minted on first launch). UI can show, copy, rotate, or disable it.
- POST /tx queues up to 8 clips: 200 started, 202 queued, 503 with Retry-After when full.
- Mute, squelch, CTCSS, audio device, and CSMA settings persist from the UI.
- Install script, systemd unit, and a desktop entry that does not hardcode a home directory.
- Cloud rows are keyed by a local device id and owned by the signed-in user. Failed tx_events are retried from an outbox.
- Alert history keeps sender, source, client, and detail across restarts.
