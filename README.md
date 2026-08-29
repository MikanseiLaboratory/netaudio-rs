# netaudio-rs

Unofficial Dante-compatible Audio-over-IP library for Rust.

`netaudio` is a **receive-first library**: the process appears as a Dante device, accepts patches from Dante Controller, and delivers PCM to the application. Clocking is an in-process overlay (`Instant` / QPC). **Windows is a first-class target**; macOS and Linux share the same API.

Protocol layouts and the three-plane device model (control / media / clock) follow [Inferno](https://github.com/teodly/inferno) (`inferno_aoip` on `dev`, plus `transmit`, `stable`, and `master`). This crate is an original MIT implementation of that model.

This project is unofficial and independent of Audinate. Dante is a trademark of Audinate Pty Ltd.

## Status

Private R&D. The product plan is in GitHub Issues. The implementable spec (tech selection, protocol layouts, public API, work order) is [`docs/IMPLEMENTATION-PLAN.md`](docs/IMPLEMENTATION-PLAN.md).

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
| Audio I/O | PCM via pull **`Device::try_read`**. Optional `set_rx_wakeup` nudge. No Stream in v1. OS audio APIs live in the app, or in [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) |
| Latency | Configurable. Minimum **4 ms** (DVS-class software clock) |
| Clock | Process-local overlay. PTPv1 listen-only, with media-packet timestamps as the Phase 2 source |

```rust
let mut settings = netaudio::Settings::default();
settings.name = "myrx".into();
settings.bind = netaudio::Bind::Ip(std::net::Ipv4Addr::new(192, 168, 1, 10));
settings.rx_channels = 2;
let dev = netaudio::Device::start(settings).await?;
let mut pcm = vec![0i32; 64 * 2];
let mut frame = netaudio::AudioFrameMut {
    media_time: netaudio::MediaTime { sample_index: 0, ns: 0 },
    sample_rate: 0,
    channels: 0,
    samples: &mut pcm,
};
let frames = dev.try_read(&mut frame)?;
```

`Device::start` requires a tokio runtime in the **caller**. `try_read` returns `0` when unpatched or nothing is due; it does not zero the caller buffer.

## Ports and firewall

Every UDP socket is bound to the configured **unicast IPv4** (never `0.0.0.0`). Open these on that interface:

| Plane | UDP | Notes |
| --- | --- | --- |
| ARC | 4440 (`alt_port+0`) | Dante Controller control |
| CMC | 8800 (`alt_port+1`) | Device advertisement |
| Info | 8700 (`alt_port+3`) | Bind. Sends to `224.0.0.231:8702` and `224.0.0.233:8708` |
| Media RX | `0x3800..=0x397F` | Unicast from TX; keepalive `[0x13,0x37]` every 250 ms |
| mDNS | 5353 | Announcer **must** source from 5353. Joins `224.0.0.251` |
| PTP | 319 / 320 | Listen-only, group `224.0.1.129`. Not shifted by `alt_port` |

Flows-control **4455** (`alt_port+2`) is a **client** in v1 (we send `0x0100` to the TX). This process does not listen on 4455.

Multicast groups used: `224.0.0.251` (mDNS), `224.0.0.231` / `224.0.0.233` (info / heartbeat), `224.0.1.129` (PTPv1).

### Linux: PTP bind

UDP 319/320 often need `CAP_NET_BIND_SERVICE` (or root). If bind is denied, the device still runs with a **media-driven** overlay (`Error::PtpBindDenied` is non-fatal at start). Example:

```
sudo setcap cap_net_bind_service=+ep /path/to/your/binary
```

### Windows

Control sockets use exclusive-address-use. mDNS uses reuse. Media sockets stay unconnected (`SIO_UDP_CONNRESET` disabled) so a TX reset does not kill the flow.

## Later

- TX channels ([#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8))
- `cpal` feature: play/capture on existing WASAPI / CoreAudio / ALSA / ASIO **host** devices ([#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9))
- User-space ASIO DLL / VST3 ([#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10))

## License

MIT
