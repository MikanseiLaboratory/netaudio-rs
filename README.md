# netaudio-rs

Unofficial Dante-compatible Audio-over-IP library for Rust.

Cross-platform `netaudio` crate. v1 is a **receive-first library**: the process appears as a Dante device, accepts patches from Dante Controller, and delivers PCM to the application. Clocking is an in-process overlay. **Windows is a first-class target**, with macOS and Linux in the same API.

Protocol layouts and the three-plane device model (control / media / clock) follow [Inferno](https://github.com/teodly/inferno) (`inferno_aoip` on `dev`, plus `transmit`, `stable`, and `master`). This crate is an original MIT implementation of that model.

This project is unofficial and independent of Audinate. Dante is a trademark of Audinate Pty Ltd.

## Status

Private R&D. Tracking RFC: [`docs/issues/001-rx-crate-plan.md`](docs/issues/001-rx-crate-plan.md). Issue index: [`docs/issues/README.md`](docs/issues/README.md).

Publishing those files as GitHub Issues: [`docs/FOLLOWUP.md`](docs/FOLLOWUP.md).

## v1 scope

| Item | v1 |
| --- | --- |
| Shape | Library (`netaudio`). One process, in-library threads |
| OS | Windows / macOS / Linux |
| Audio I/O | PCM via callback / Stream / ring buffer. OS audio APIs live in the app (or a later `cpal` feature) |
| Latency | Configurable. Minimum **4 ms** (DVS-class software clock) |
| Clock | Process-local overlay (`Instant` / QPC). PTPv1 listen-only + media-packet timestamps |

## Later

- `cpal` feature: play/capture on existing WASAPI / CoreAudio / ALSA / ASIO **host** devices
- TX channels (`tx_latency` ≥ 4 ms)
- User-space ASIO DLL / VST3 (installable audio host). Separate issues.

## License

MIT
