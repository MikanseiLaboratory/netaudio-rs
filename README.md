# netaudio-rs

Unofficial Dante-compatible Audio-over-IP library for Rust.

Receive-first, library-first, cross-platform. No external PTP daemon.
Not a fork of [Inferno](https://github.com/teodly/inferno).

This project is **not affiliated with, authorized, or endorsed by Audinate**.
Dante is a trademark of Audinate Pty Ltd.

## Status

Private R&D. Plan is in [`docs/issues/001-rx-crate-plan.md`](docs/issues/001-rx-crate-plan.md) until a follow-up agent opens GitHub Issues.

Follow-up (create private repo, register deploy key, push, open issues): [`docs/FOLLOWUP.md`](docs/FOLLOWUP.md).

## Non-goals (v1)

- Virtual soundcard (ALSA / WDM / ASIO driver)
- External daemons (`statime`, `ptp4l`, `usrvclock`)
- AES67 / SMPTE ST 2110-30
- Dante Domain Manager

## License

MIT
