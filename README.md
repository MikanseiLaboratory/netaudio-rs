# netaudio-rs

Unofficial Dante-compatible Audio-over-IP library for Rust.

`netaudio` is a **receive-first library**: the process appears as a Dante device, accepts patches from Dante Controller, and delivers PCM to the application. Clocking is an in-process overlay (`Instant` / QPC). **Windows is a first-class target**; macOS and Linux share the same API.

Protocol layouts and the three-plane device model (control / media / clock) follow [Inferno](https://github.com/teodly/inferno) (`inferno_aoip` on `dev`, plus `transmit`, `stable`, and `master`). This crate is an original MIT implementation of that model.

This project is unofficial and independent of Audinate. Dante is a trademark of Audinate Pty Ltd.

## Status

Private R&D. The plan is in GitHub Issues.

| GitHub | Topic |
| --- | --- |
| [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2) | Tracking RFC — Inferno-aligned, Windows first-class RX crate |
| [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3) | Protocol codecs (ARC / CMC / flows-control / media header) |
| [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) | In-process overlay clock (PTPv1 listen-only + media timestamps) |
| [#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) | Interface-bound sockets, multicast, mDNS 5353, PTP ports |
| [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) | Control plane — appear in Dante Controller |
| [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7) | Media RX — subscribe, ring buffer, `AudioBlock` |
| [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8) | TX — unicast, `tx_latency` ≥ 4 ms |
| [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) | `cpal` feature — existing OS devices |
| [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) | Audio host — user-space ASIO / VST3 |

v1 is Phase 3: control plane + media RX + overlay clock locked to PTPv1.

## v1

| Item | v1 |
| --- | --- |
| Shape | Library (`netaudio`). Control, media, and clock run in-process |
| OS | Windows / macOS / Linux |
| Audio I/O | PCM via callback / Stream / ring buffer. OS audio APIs live in the app, or in [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) |
| Latency | Configurable. Minimum **4 ms** (DVS-class software clock) |
| Clock | Process-local overlay. PTPv1 listen-only, with media-packet timestamps as the Phase 2 source |

## Later

- TX channels ([#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8))
- `cpal` feature: play/capture on existing WASAPI / CoreAudio / ALSA / ASIO **host** devices ([#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9))
- User-space ASIO DLL / VST3 ([#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10))

## License

MIT
