# netaudio-rs — implementation plan

This is the **single source of implementation truth**. An implementer (human or LLM) with this file, `README.md`, and the crate can ship v1.

- Language: English (wire names, field names, and code identifiers).
- License of this crate: MIT. Write original source from this document and public captures.
- Protocol facts below are reverse-engineered layouts (packet offsets, opcodes, TXT keys). Hex examples are from public captures (Dante Controller / hardware / [network-audio-controller](https://github.com/chris-ritsen/network-audio-controller), Unlicense).

GitHub Issues #2–#10 exist on the private repo. This Cloud Agent token cannot read Issue bodies (HTTP 403). Coverage is reconstructed from README titles, the original RFC in git history (`docs/ISSUE-001-rx-crate-plan.md` at `7613c55`), and the four specialist reviews that brushed up the draft.

---

## 0. How to use this document

1. Read **§1–§5** (product, issues, tech, requirements). Do not reopen locked decisions.
2. Implement in **§13 work-item order**. Each item lists files, tests, and definition of done.
3. When encoding/decoding packets, follow **§8** exactly (endianness, offset base, 1-based IDs).
4. When binding sockets, follow **§9**. Never bind `0.0.0.0`.
5. When touching time or PCM, follow **§10–§11**.
6. If a capture disagrees with this document, **prefer a pcap from Dante Controller + hardware**, then update this file in the same PR.

Keep original source in this tree. Protocol codecs follow this document and public captures. Keep `edition = "2024"`. Overlay clock stays in-process (`Instant` / QPC). mDNS `mf` / `model` / device `name` use Mikansei / netaudio defaults.

---

## 1. Evaluation (issues + draft + subagent review)

### 1.1 Issue map

| Issue | README title | v1? | This plan |
| --- | --- | --- | --- |
| [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2) | Tracking RFC — Windows first-class RX crate | contract | this document |
| [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3) | Protocol codecs (ARC / CMC / flows-control / media header) | yes | §8, W1 |
| [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) | In-process overlay clock (PTPv1 listen-only + media timestamps) | yes | §10, W5/W7 |
| [#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) | Interface-bound sockets, multicast, mDNS 5353, PTP ports | yes | §9, W2 |
| [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) | Control plane — appear in Dante Controller | yes | §8–§9, W3–W4 |
| [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7) | Media RX — subscribe, ring buffer, `AudioBlock` | yes | §7, §11, W6 |
| [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8) | TX — unicast, `tx_latency` ≥ 4 ms | later | §15.1 |
| [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) | `cpal` feature — existing OS devices | later | §15.2 |
| [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) | Audio host — user-space ASIO / VST3 | far | §15.3 |

**v1 = Phase 3 = issues #2–#7 done.** TX / cpal / ASIO are specified now so later work does not fight the API, but they are not implemented in v1.

### 1.2 Draft errors that this plan corrects

Four specialist reviews (protocol, clock/media, sockets/mDNS, API) found these mistakes in the first draft. **Do not reintroduce them.**

| Draft claim | Correction |
| --- | --- |
| ARC `0x3300` is a “clock domain mismatch stub” | `0x3300` is **RX unicast media port ranges** (four `u16`). Clock mismatch is subscription status **`0x0000001B`**. Hardware reply: `3800 397f 3980 39ff`. Bind media RX in `0x3800..=0x397F`. |
| String offsets are unspecified | Offsets are **from UDP payload byte 0** (packet start), not from content. `0` = absent. |
| Pagination is vague | Request `content[2..4)` is **1-based start**. Response `[0]=space, [1]=count`. More pages: **`opcode2 = 0x8112`**. |
| ARC `0x1000` is “flags + a few counts” | **36-byte** body; `unknown2 = 4`, `unknown4 = 8`, trailing `1,1` + 12 zero bytes. `max_channels_in_flow = 8` even when `tx=0`. |
| Socket descriptor magic `0x8002` | Wire value is **`0x0802`**. |
| Info heartbeat `start_code` unspecified | Device-info **`0xFFFF`** → `:8702`. Heartbeat **`0xFFFE`** → `:8708`. |
| Info mcast required for DC device list | **No.** List = mDNS `_netaudio-arc` + ARC. Info mcast is still **Phase 1 must** to stop DC flooding `:8700` (`board[0xBB] = 0x1F`) and to feed clock/peaks UI. |
| High-port mDNS **announce** as 5353 fallback | Illegal (RFC 6762). High port is only for **legacy unicast queries**. Announcer **must** source from 5353. |
| `netdev` is Unix-only | False. Use it for Windows friendly names, MAC, netmask, gateway. |
| Overlay `local - last_sync` in `u64` | All diffs are **`i64` wrapping**. See §10. |
| 200 ms clock clamp is enough at 4 ms latency | 200 ms is an **outlier reject**. Audio-safe slew is **50 µs/step when locked**. |
| Callback `FnMut(AudioBlock<'_>)` on `Device: Sync` | Pull **`try_read`** is the stable API. Optional **wakeup** `Fn() + Send + Sync`. No `Stream` in v1. |
| Media-driven servo at packet rate | Decimate media observations to **≤ 8 Hz**. |
| Bind helper defaulting to `0.0.0.0` | Bind every socket to the configured unicast IPv4. See §9. |
| PTP multicast group omitted | **`224.0.1.129`**. Subdomain default **`_DFLT`**. |
| Delay_Req “just skip” | Listen-only is **one-way**; path delay sits in `shift`; covered by `rx_latency ≥ 4 ms`. |

### 1.3 Locked answers to remaining trade-offs

| Topic | Decision |
| --- | --- |
| Media poll | `polling` 3 (epoll / kqueue / IOCP). Not busy-wait. Not `mio` in v1. |
| Device ID | If NIC MAC known: `[0,0] + mac[0..6]`. Else `[0,0] + ipv4 + process_id BE`. Overridable. |
| NIC crate | `netdev` (MIT, Win/macOS/Linux). |
| mDNS announcer | **Custom** responder on a dedicated OS thread. `mdns-sd` is not default (binds `0.0.0.0`, drops empty TXT). |
| RX API | Pull `try_read` stable; wakeup optional; data callback not v1. |
| Info mcast | Phase 1 **must** (board + product + 1 Hz heartbeat, `0xBB=0x1F`). |
| `_netaudio-dbc` | Skip on RX-only v1. |
| `_netaudio-chan` | Do not **announce** when `tx_channels=0`. **Query** it in Phase 2 to resolve TX channels. |
| Media header byte 0 | RX **ignore**. TX later: write **`0x02`** (observed on working transmitters). |
| Sample rates | Only 44100 / 48000 / 88200 / 96000. Mismatch → no `0x0100` / TX error `0x0301`. |
| Public modules | `protocol`, `net`, `media` are **`pub(crate)`**. Do not freeze wire types as public API. |
| Media RX port | Bind in advertised range `0x3800..=0x397F`. If busy, try next port in range; last resort ephemeral (still report actual port in `0x3200`). |
| PTP I/O | Dedicated **clock thread** (recv 319/320, timestamp `t2` immediately). Not tokio. Overlay published via seqlock. |
| `thiserror` | 2.x |
| `socket2` | 0.5 with feature `all` (Unix `SO_REUSEPORT`). |

---

## 2. What v1 is

A **library** (`netaudio`) that:

1. Appears on a Dante network as a receive device (Dante Controller can see it and patch to it).
2. Delivers PCM to the application as left-justified `i32` samples plus media time.
3. Keeps an **in-process overlay clock** locked to PTPv1 when possible, else to media-packet timestamps.

Windows is a first-class target. macOS and Linux share the same public API.

v1 delivers PCM to the application. OS playback, a virtual sound card, and a PTP daemon are later work (#9 / #10).

### 2.1 Later than v1 (see §15)

- Virtual sound card (ALSA plugin, WDM, user-space ASIO DLL) — #10
- `cpal` playback/capture — #9
- TX channels — #8 (`tx_channels` must be 0; `tx_latency` still validated)
- Dante Domain Manager
- AES67 / ST 2110-30
- PTPv2, PTP leader, Delay_Req/Delay_Resp
- IPv6
- Multiple instances on one IP; use `alt_port` when sharing an address
- OS clock steering

### 2.2 App-side uses

Write a file, bridge to another protocol, later feed `cpal`, DSP, meters. The crate stays a PCM + media-time library; OS audio APIs stay in the app or in #9.

---

## 3. Requirements

### 3.1 Functional — Phase 1 (appear in Dante Controller)

Issues #5 + #6 + codec subset of #3.

- Bind **every** UDP socket to the configured unicast IPv4. Reject `0.0.0.0`.
- Advertise `_netaudio-arc._udp` and `_netaudio-cmc._udp` (mDNS, IF-bound, source port 5353).
- Serve ARC (4440 or `alt+0`) and CMC (8800 or `alt+1`).
- CMC `0x1001` → 22-byte `DeviceAdvertisement`.
- ARC minimum: `0x1000`, `0x1002`, `0x1003`, `0x1100` (110 zero bytes), `0x1102` (94 zero bytes or property directory), `0x2000`/`0x2010`/`0x2200` empty pages, `0x2320` → `opcode2=0x0030`, `0x3000` RX list, `0x3010` parse+ACK (record only), `0x3200` empty page, `0x3300` port ranges.
- Info bind 8700 (`alt+3`); send board+product to `224.0.0.231:8702`; heartbeat 1 Hz to `224.0.0.233:8708`; `board[0xBB]=0x1F`.
- `Device::bound_ports()` returns every bound UDP port.
- Unknown ARC `opcode1`: log, **no reply**, never panic.

**Acceptance:** Dante Controller device list shows name, RX channel count, IP.

### 3.2 Functional — Phase 2 (media RX)

Issues #3 remainder + #7 + media-driven clock from #4.

- On ARC `0x3010` with non-zero string offsets: resolve TX via mDNS, send flows-control `0x0100` to TX `:4455`, bind unicast media UDP in `0x3800..=0x397F`, keepalive `[0x13,0x37]` every 250 ms to last media source.
- Parse 9-byte media header; write BE PCM into a timestamped ring; promote to left-justified `i32`.
- App reads at `overlay_now − rx_latency`. Unpatched: `try_read` returns `0` (no silence fill).
- Unsubscribe (zero offsets) → `0x0101`, drop socket.
- Sample-rate mismatch: do not request the flow / treat as error.

**Acceptance:** DC patch → `try_read` yields PCM. Drop patch → stops.

### 3.3 Functional — Phase 3 (PTPv1 overlay)

Issue #4.

- Listen PTPv1 UDP 319/320 on bound IF, join `224.0.1.129`. Windows: no admin. Linux/macOS: bind failure is `PtpBindDenied`; **do not abort start**; fall back to media-driven clock.
- Listen-only: Sync + Follow_Up. No Delay_Req. No OS clock steer.
- Filter `versionPTP == 1` and subdomain default `_DFLT`.
- DC must not show persistent clock-domain mismatch (`0x001B`) after PTP lock + info-mcast clock TLVs.
- `rx_latency = 4 ms` continuous RX does not glitch under normal GbE load (manual soak).

**Acceptance:** hardware + DC, ~1 hour continuous RX. CI does not require hardware.

### 3.4 Cross-platform non-functional

- Multicast: `IP_MULTICAST_IF` + `join_multicast_v4(group, iface_ipv4)` for mDNS and PTP. **Do not** join 224.0.0.231 / 224.0.0.233 (info is send-only).
- mDNS 5353: Windows `SO_REUSEADDR`; Unix `SO_REUSEADDR` + `SO_REUSEPORT`. Failure → `MdnsPortInUse`.
- Media RX: unconnected UDP; Windows `SIO_UDP_CONNRESET = FALSE`.
- Media `SO_RCVBUF` target 2 MiB (log clamp).
- Media thread: best-effort realtime (Windows `THREAD_PRIORITY_TIME_CRITICAL`); failure is OK.
- IPv4 only.
- No panics on malformed packets on media/control paths.

### 3.5 Tests (automated)

- Codec round-trips from hex fixtures (field names we chose).
- Pagination (`0x8112`, 1-based start).
- Overlay wrap, slew, 200 ms outlier, EMA.
- Bind: unspecified IP rejected; multicast join uses iface.
- Loopback: fake TX + one `Device` (see W8). No hardware in CI.

### 3.6 Legal / hygiene

- MIT. Original source in this tree.
- README disclaimer stays (unofficial, Audinate trademark, patents).
- Display `name` / mDNS `mf` / `model` must not impersonate Dante hardware. Default manufacturer `Mikansei`, model `netaudio`.
- Info-mcast 8-byte `vendor` field: existing controllers often require ASCII starting with `Audinate` (8 chars). This is a **protocol compatibility tag**, not a product claim. Setting: `vendor_tag: [u8; 8]` default `*b"Audinate"`. Document in README.

---

## 4. Tech stack (v1)

Do not enable `cpal` by default.

```toml
[package]
name = "netaudio"
version = "0.0.0"
edition = "2024"
rust-version = "1.98"
license = "MIT"
publish = false

[dependencies]
tokio = { version = "1", default-features = false, features = ["net", "time", "sync", "macros", "rt"] }
log = "0.4"
thiserror = "2"
socket2 = { version = "0.5", features = ["all"] }
byteorder = "1.5"
polling = "3"
netdev = "0.32"

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = ["Win32_Networking_WinSock", "Win32_System_IO", "Win32_System_Threading"] }

[dev-dependencies]
hex = "0.4"
tokio = { version = "1", features = ["net", "time", "sync", "macros", "rt", "rt-multi-thread"] }

[features]
default = []
cpal = []
```

| Slot | Choice | Why |
| --- | --- | --- |
| async control | tokio 1 (already in crate) | timeouts, cancel, UDP |
| sockets | socket2 0.5 `all` | IF bind, reuse, multicast, Windows |
| media poll | polling 3 | epoll/kqueue/IOCP, no mio |
| bytes | byteorder + internal `BeBuf` | explicit layouts, fixture-friendly |
| log | log 0.4 | app picks backend |
| errors | thiserror 2 | typed public errors |
| NIC | netdev 0.32 | Win friendly name, MAC, mask, gateway |
| WinSock extras | windows-sys | `SO_EXCLUSIVEADDRUSE`, `SIO_UDP_CONNRESET`, thread priority |
| mDNS | original responder | Dante TXT + empty records + IF bind |
| clock | `std::time::Instant` overlay | QPC on Windows; no daemon |

**Stack for v1:** tokio (features listed above), socket2, polling, byteorder, log, thiserror, netdev, windows-sys on Windows. Transitive `libc` via socket2/polling is allowed.

---

## 5. Architecture

```
application
    │  Sample = i32, try_read, Settings, Device
    ▼
[device]   Tokio: ARC, CMC, info mcast, subscribe orchestration
    │
    ├─► [net]     IF-bound UDP, multicast, mDNS thread, NIC resolve
    ├─► [media]   OS thread: flow UDP, keepalive, planar ring
    ├─► [clock]   OS thread: PTPv1 listen, seqlock overlay
    └─► [protocol]  pure ser/de (no I/O)
```

Dependency direction (acyclic):

```
protocol          (leaf)
    ↑
clock, net
    ↑
media
    ↑
device  → tokio
```

Hard rules:

- `media` never calls tokio.
- `protocol` never calls `net`.
- `clock` never steers OS time.
- Public crate surface is `lib.rs` + `device` types only.

Threads:

| Thread | Work |
| --- | --- |
| Caller tokio runtime | ARC, CMC, info, flows-control **client**, mDNS **querier** |
| mDNS announcer OS thread | probe + announce + answer on `:5353` |
| media OS thread | `polling` of flow sockets, ring write, keepalive |
| clock OS thread | PTP 319/320 recv, overlay seqlock update |
| (none extra) | `try_read` runs on the **caller’s** thread |

Realtime: media thread best-effort. `try_read` is wait-free aside from a seqlock retry.

---

## 6. Public API (v1 contract)

```rust
pub type Sample = i32; // left-justified; see §11.4

#[non_exhaustive]
pub enum Bits { B16, B24, B32 }

#[non_exhaustive]
pub enum Bind {
    Ip(std::net::Ipv4Addr),
    Interface(String),
}

pub struct Settings {
    pub name: String,                 // one DNS label, 1..=31 bytes
    pub bind: Bind,
    pub rx_channels: u16,             // >= 1
    pub tx_channels: u16,             // v1: must be 0
    pub sample_rate: u32,             // 44100|48000|88200|96000
    pub bits: Bits,                   // default B24
    pub rx_latency: std::time::Duration, // default 10ms, min 4ms
    pub tx_latency: std::time::Duration, // same floor; unused until #8
    pub device_id: Option<[u8; 8]>,
    pub process_id: u16,
    pub alt_port: Option<u16>,
    pub rx_channel_names: Option<Vec<String>>,
    pub manufacturer: String,         // mDNS mf; default "Mikansei"
    pub model: String,                // mDNS model; default "netaudio"
    pub board: String,                // mDNS router_info; default "netaudio-rs"
    pub vendor_tag: [u8; 8],          // info-mcast vendor; default *b"Audinate"
    pub ptp_subdomain: [u8; 16],      // default b"_DFLT" + NULs
}

impl Default for Settings { /* 48 kHz, B24, 10 ms, 1 RX named "Rx 1", bind required */ }
impl Settings {
    pub fn validate(&self) -> Result<(), Error>;
}

pub struct Device { /* private Arc inner; Send + Sync; !Clone */ }

impl Device {
    pub async fn start(settings: Settings) -> Result<Self, Error>;
    pub fn try_read(&self, dst: &mut AudioFrameMut<'_>) -> Result<usize, Error>;
    pub fn set_rx_wakeup(&self, f: impl Fn() + Send + Sync + 'static);
    pub fn bound_ports(&self) -> BoundPorts;
    pub fn clock_status(&self) -> ClockStatus;
    pub async fn shutdown(self) -> Result<(), Error>;
}
// Drop: signal stop, unpark threads, do NOT block_on (deadlock on tokio).

pub struct AudioFrameMut<'a> {
    pub media_time: MediaTime,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: &'a mut [Sample], // interleaved, len >= frames * channels
}

pub struct MediaTime {
    pub sample_index: u64, // wrapping; first sample of this block
    pub ns: u64,           // wrapping overlay ns of that sample
}

pub struct BoundPorts {
    pub arc: u16,
    pub cmc: u16,
    pub flows_control: Option<u16>, // None in RX-only v1 (we do not listen)
    pub info: u16,
    pub mdns: Option<u16>,          // 5353 if announcer bound
    pub ptp_event: Option<u16>,     // 319 if bound
    pub ptp_general: Option<u16>,   // 320 if bound
    pub media: Vec<u16>,
}

#[non_exhaustive]
pub enum ClockStatus {
    Unlocked,
    MediaDriven,
    PtpLocked,
}

#[non_exhaustive]
pub enum Error {
    InvalidSettings { reason: String },
    UnspecifiedAddress,
    InterfaceNotFound { name: String },
    InterfaceHasNoIpv4 { name: String },
    PortInUse { port: u16, role: &'static str },
    MdnsPortInUse,
    PtpBindDenied { port: u16 },
    Stopped,
    Io(std::io::Error),
}
```

Rules:

- `start` requires a tokio runtime in the **caller**. Do not spawn a hidden runtime.
- `try_read` returns **frames written** (0 = nothing ready: unpatched, not yet due, or unlocked with no media). It does **not** zero the caller’s buffer.
- `set_rx_wakeup` is a nudge only. The app still calls `try_read`. Single consumer.
- `validate`: `rx_latency` and `tx_latency` ≥ 4_000_000 ns; `name` 1..=31 bytes, one label `[A-Za-z0-9-]` (no dots, no spaces); `tx_channels == 0` in v1; sample rate in the set; `rx_channels >= 1`; `rx_channel_names` if `Some` must match `rx_channels`.
- `alt_port` shifts ARC/CMC/flows/info only. **Never** shift 5353, 319, 320.
- `process_id` default: low 16 bits of OS pid, overridable.

---

## 7. Crate layout

Keep existing roots; add `net`. Flip today’s `pub mod` to crate-private except re-exports.

```
src/
  lib.rs
  device.rs                 // re-export Settings, Device, Error
  device/
    mod.rs
    settings.rs
    error.rs
    arc.rs                  // tokio ARC server
    cmc.rs
    info_mcast.rs
    subscribe.rs
  clock.rs
  clock/
    mod.rs
    overlay.rs              // seqlock + formula
    ptp.rs                  // listen thread
  media.rs
  media/
    mod.rs
    ring.rs                 // planar timestamped ring
    rx.rs                   // media thread
    keepalive.rs
  protocol.rs
  protocol/
    mod.rs
    buf.rs                  // BeBuf
    req_resp.rs
    arc.rs
    cmc.rs
    flows_control.rs
    info_mcast.rs
    media.rs                // 9-byte header
    pcm.rs                  // promote/demote
    mdns.rs                 // DNS message builder
    ptp_v1.rs
  net.rs
  net/
    mod.rs
    udp.rs                  // bind helper
    iface.rs                // netdev resolve
    mdns.rs                 // announcer thread + querier
    ports.rs
    windows.rs              // cfg(windows) sockopts
```

Tests:

```
tests/
  settings.rs
  bind.rs
  protocol_arc.rs
  protocol_cmc.rs
  protocol_flows.rs
  protocol_media.rs
  protocol_mcast.rs
  protocol_ptp.rs
  clock_overlay.rs
  ring.rs
  loopback_rx.rs
  fixtures/
    arc/*.hex
    cmc/*.hex
    flows/*.hex
    media/*.hex
    mcast/*.hex
    ptp/*.hex
    mdns/*.hex
```

Each hex file: raw bytes plus a first-line comment `# provenance: DC 4.x / device X / date` or `# synthetic: field names ...`.

---

## 8. Protocol specification

Conventions:

- IPv4 UDP, **big-endian**.
- Channel IDs **1-based**. `0` in a channel-id slot = unused.
- String offsets: **`u16` from UDP payload byte 0**. `0` = absent. Strings are ASCII + NUL.
- `HEADER_RR = 10`. `HEADER_MCAST = 32`.
- Echo `start_code`, `seqnum`, `opcode1` on every reply.
- `opcode2`: `0` = request, `1` = OK, `0x8112` = OK + more pages, other = error.
- Unknown ARC `opcode1`: log, no reply.

### 8.1 Ports

| Role | Default | `alt_port = A` |
| --- | --- | --- |
| ARC | 4440 | A+0 |
| CMC | 8800 | A+1 |
| flows-control dest (TX) / server (our TX later) | 4455 | A+2 |
| info bind (unicast recv + mcast send src) | 8700 | A+3 |
| info dest | 224.0.0.231:8702 | same dest |
| heartbeat dest | 224.0.0.233:8708 | same dest |
| mDNS | 224.0.0.251:5353 | never shifted |
| PTP event / general | 224.0.1.129:319 / :320 | never shifted |
| media RX unicast | bind in `0x3800..=0x397F` | same range |

Do not bind 8702/8708. Do not bind 4455 in RX-only v1.

### 8.2 req/resp header (ARC, CMC, flows-control)

| off | len | field |
| --- | --- | --- |
| 0 | 2 | `start_code` |
| 2 | 2 | `total_length` = **entire UDP payload** |
| 4 | 2 | `seqnum` |
| 6 | 2 | `opcode1` |
| 8 | 2 | `opcode2` |
| 10 | n | content |

`start_code`:

- ARC: **echo the client**. Common values `0x2729`, `0x27FF`, `0x2801`, `0x2809`.
- CMC: often `0x1200`; echo client.
- flows-control client: always send **`0x1102`**.

### 8.3 Pagination (ARC `0x2000`, `0x2010`, `0x2200`, `0x3000`, `0x3200`)

Request content (typical):

| off | field |
| --- | --- |
| 0–1 | unused / space hint |
| 2–3 | **1-based start index** (`0` is invalid) |

`start_0 = be_u16(content[2..4]) - 1`.

Response content:

| off | field |
| --- | --- |
| 0 | `space` u8 (slots this page; channels `min(n,32)`, flows `min(max,16)`) |
| 1 | `count` u8 (items written) |
| 2 | items |

- `0x3000` / `0x2000` / `0x2010`: items **are** descriptors.
- `0x2200` / `0x3200`: items are **`u16` packet offsets** to descriptors (`space` slots reserved even if `count` smaller).
- More remain or packet would exceed ~800–900 bytes → `opcode2 = 0x8112`.
- Empty: `space=0, count=0`, `opcode2=1`.

### 8.4 ARC opcodes (RX-only)

| opcode1 | Phase 1 | Body |
| --- | --- | --- |
| `0x1000` | MUST | 36-byte counts |
| `0x1002` | MUST | hostname + NUL |
| `0x1003` | MUST | 38-byte names header + strings |
| `0x1100` | MUST stub | 110 zero bytes |
| `0x1102` | MUST stub | 94 zero bytes (or hardware property directory) |
| `0x2000` | MUST empty | TX factory names |
| `0x2010` | MUST empty | TX friendly names |
| `0x2013` | later / `0xFFFF` | rename TX |
| `0x2200` | MUST empty | TX flows |
| `0x2201`/`0x2202` | later / `0xFFFF` | mcast TX create/delete |
| `0x2320` | SHOULD | `opcode2=0x0030`, empty |
| `0x3000` | MUST | RX channels |
| `0x3001` | SHOULD | rename RX + info channel-change |
| `0x3010` | MUST parse+ACK | subscribe; act in Phase 2 |
| `0x3014` | SHOULD | unsub one RX (NAC); content channel id at `[4..6)` |
| `0x3200` | MUST empty P1; fill P2 | RX flows |
| `0x3300` | MUST | 8-byte port ranges |

ARC `0x1001` is **not** CMC `0x1001`. If a client sends ARC `0x1001` (set name), SHOULD treat as rename; do not confuse with advertisement.

#### 8.4.1 `0x1000` counts — 36 bytes

| off | type | RX-only value |
| --- | --- | --- |
| 0 | u8 | 0 |
| 1 | u8 | flags2 = `0x00` (bit4 TX rename, bit5 TX mcast; both off) |
| 2 | u16 | tx_channels = 0 |
| 4 | u16 | rx_channels = N |
| 6 | u16 | 4 |
| 8 | u16 | max_channels_in_flow = **8** |
| 10 | u16 | 8 |
| 12 | u16 | max_tx_flows = 32 |
| 14 | u16 | max_rx_flows = 32 |
| 16 | u16 | N (tx+rx) |
| 18 | u16 | 1 |
| 20 | u16 | 1 |
| 22 | 12 B | zeros |

#### 8.4.2 `0x1003` names — 38-byte header then strings

All header fields `u16` BE, content offsets:

| content off | value |
| --- | --- |
| 0,2,4 | 0 |
| 6 | board_name packet offset |
| 8 | revision packet offset (ASCII e.g. `0.0.0`) |
| 10 | 0 |
| 12 | friendly hostname offset |
| 14 | factory hostname offset |
| 16 | friendly hostname offset (same as 12) |
| 18–29 | zeros |
| 30 | `0x2729` |
| 32 | 0 |
| 34 | `0x1102` |
| 36 | 0 |

Packet offset = `10 + position_in_content` for any string that lives after this header. Factory hostname: `{short}-{hex device_id}`, ≤31.

#### 8.4.3 Shared PCM descriptor — 16 bytes

| off | type | value |
| --- | --- | --- |
| 0 | u32 | sample_rate |
| 4 | u8 | 1 |
| 5 | u8 | 1 |
| 6 | u16 | bits (16/24/32) |
| 8 | u16 | `0x0400` |
| 10 | u16 | bits |
| 12 | u16 | bits |
| 14 | u16 | `pcm_type` = `0x000E` |

#### 8.4.4 `0x3000` RX channel descriptor — 20 bytes

| off | type | value |
| --- | --- | --- |
| 0 | u16 | channel_id 1-based |
| 2 | u16 | `0x0006` |
| 4 | u16 | offset → PCM descriptor |
| 6 | u16 | TX channel name offset (`0` if unsub) |
| 8 | u16 | TX hostname offset (`0` if unsub) |
| 10 | u16 | local RX friendly name offset |
| 12 | u32 | subscription status |
| 16 | u32 | 0 |

Status:

| u32 | meaning |
| --- | --- |
| `0x00000000` | none |
| `0x00000001` | unresolved / in progress |
| `0x00000008` | establishing |
| `0x01010009` | receiving unicast |
| `0x0101000A` | receiving multicast |
| `0x0000001B` | clock domain mismatch |
| `0x00000014` | no more TX flows |

Phase 1 with a recorded but not-yet-flowing sub: `0x00000001`. Phase 2 after packets: `0x01010009`.

#### 8.4.5 `0x3010` subscribe

`space, count` then `count` records of 6 bytes: `local_id u16`, `tx_name_off u16`, `tx_host_off u16`. Honor **`count` (byte 1)**, not `space` (DC often sends `space=0x20` with trailing zeros). Both offsets 0 = unsubscribe that channel. Reply: `opcode2=1`, **empty content**.

#### 8.4.6 `0x3300` port ranges

Request: empty content (`total_length=10`).

Golden (public capture):

```
req:  2729000a033c33000000
resp: 27290012033c330000013800397f398039ff
```

Response content 8 bytes: `0x3800, 0x397F, 0x3980, 0x39FF`.

### 8.5 CMC `0x1001` — 22 bytes

| off | type | value |
| --- | --- | --- |
| 0 | u16 | process_id |
| 2 | [u8;8] | device_id |
| 10 | u16 | 1 |
| 12 | u16 | 0 |
| 14 | [u8;4] | IPv4 |
| 18 | u16 | info bind port |
| 20 | u16 | 0 |

### 8.6 flows-control client (Phase 2)

We are the **client**. We do not listen on 4455 in v1.

Always `start_code=0x1102`. Seqnum increments from 1. Timeout 3 s. Match reply by `opcode1` + `seqnum`.

#### `0x0100` request_flow

Let `n = nchan`, `H = 10`.

Content:

| content off | field |
| --- | --- |
| 0 | `strings_off` u16 **packet-relative** = `48 + 2n` |
| 2 | sample_rate u32 |
| 6 | bits_per_sample u32 (16/24/32, **not bytes**) |
| 10 | `1` u16 |
| 12 | `n` u16 |
| 14 | socket_desc offset packet-relative (8-aligned after strings) |
| 16 | `n` × u16 TX channel ids (`0` unused, else 1-based) |
| 16+2n | extra offset `0x1C + 2n` (packet-relative to `0x0A00` blob) |
| 18+2n | `0x0A00` |
| 20+2n | `0x0002` |
| 22+2n | `fpp` u16 |
| 24+2n | rx_flow_name offset packet-relative |
| 26+2n | 12 zero bytes |
| 38+2n | `hostname\0` `flow_name\0` pad so **packet offset % 8 == 0**, then `08 02`, port u16, IPv4 |

`len(content before strings) == 38+2n`.

OK reply: content `[0..6)` = `FlowHandle`. Errors: `opcode2` `0x0103` expired, `0x0315` too many TX flows, `0x0301` rate mismatch.

Public capture (request_flow, n=4, bits=24, fpp=16):

```
1102005000000100000000380000bb8000000018000100040048000100000000000000240a000002001000430000000000000000000000004133322d303030303031003100000000080238010afe4e0b
```

#### `0x0102` update

`handle[6] + nchan u16 + ids`.

#### `0x0101` stop

`handle[6]` only.

### 8.7 Media packet

| off | RX | TX (#8) |
| --- | --- | --- |
| 0 | ignore | write `0x02` |
| 1–4 | seconds u32 BE | `sample_index / rate` |
| 5–8 | index-in-second u32 BE | `sample_index % rate` |
| 9+ | interleaved BE PCM | same |

```
sample_index = (seconds as u64).wrapping_mul(rate as u64).wrapping_add(index as u64)
```

Keepalive: payload `[0x13, 0x37]`, every 250 ms, `send_to(last_media_source)`. Do not send until a media packet has revealed the source. If multiple intervals were skipped, send once. Stagger flows by `250ms / max(1, n_flows)`. TX expires a flow after ~4 s without keepalives.

FPP: request `min(advertised_max_from_mDNS, mtu_limit, floor(rate * rx_latency_ns / 4 / 1e9))`, still ≥ advertised min and ≥ 2. Typical 4–32. MTU: `(1400 - 9) / (nchan * bytes_per_sample)`.

### 8.8 Info multicast

32-byte header:

| off | field |
| --- | --- |
| 0 | start_code `0xFFFF` (info) or `0xFFFE` (heartbeat) |
| 2 | total_length = 32 + content |
| 4 | seqnum |
| 6 | process_id |
| 8 | device_id [8] |
| 16 | vendor [8] = `vendor_tag` (default `Audinate`) |
| 24 | opcode [8] |
| 32 | content |

Bind 8700 on device IPv4. `set_multicast_if_v4`. TTL 1. **Send-to** destinations; do not join 231/233.

| dest | start | opcode[8] | when | content |
| --- | --- | --- | --- | --- |
| :8702 | FFFF | `07 2a 00 60 00 00 00 00` | start + req `…00 61…` | board 200 B |
| :8702 | FFFF | `07 2a 00 c0 00 00 00 00` | start + req `…00 c1…` | product 336 B |
| :8708 | FFFE | `00 08 00 01 10 00 00 00` | every 1 s | heartbeat TLVs |
| :8702 | FFFF | `07 2a 00 20 00 00 00 00` | req `…00 21…` | clock stats (Phase 3) |
| :8702 | FFFF | `07 2a 00 11 00 00 00 00` | req `…00 13…` | network info |
| :8702 | FFFF | `07 2a 01 02 00 00 00 00` | RX rename | channel bitmask |

Match requests on bytes 24–31: `[0x07, _, 0, type, 0, 0, 0, _]`. Reply type ≈ request type − 1 for `61→60`, `c1→c0`, `21→20`, `13→11`.

**Board 200 B (minimum):**

- `[0..4)` FW `04 01 00 06`
- `[4..8)` HW `04 01 00 03`
- `[0xBB] = 0x1F` (**must**, or DC floods `:8700` at ~1 Hz)
- ASCII board name at 12 (max 8) and `0x38` (max 16), zero-filled

**Product 336 B:** manufacturer at 0 (8) and `0x2C` (16), board at 8 (8), model at `0xAC` (16).

**Heartbeat TLVs** (concatenated; skip if empty):

Each: `u16 rec_len`, `u16 type`, body.

- `0x8001` clock ppm — rec_len 16: `u16 4, u16 4, u16 seq, u16 0, i32 ppb` with `ppb = (freq_scale * 1e9) as i32`. Skip if unlocked.
- `0x8002` peaks — optional v1 (zeros OK).
- `0x8003` RX latency samples — optional v1.

**Channel-change:** opcode `[0x07, 0x2a, 0x01, 0x02, 0,0,0,0]`. Content: `u16 mask_len_bytes`, then bits; channel `i` → byte `2 + i/8`, bit `i%8`. Empty: `00 01 00`.

**Network info (`0x13`):** `00 01 00 00 00 00`, `u16 link_mbps`, `u16 1`, mac[6], ip[4], mask[4], gw[4], dns[4] (dns may be 0).

### 8.9 mDNS

Announce **only**:

1. `{hostname}._netaudio-arc._udp.local.` SRV = ARC port, A = device IPv4. TTL **4500** s.  
   TXT (one length-prefixed string each, max 255):  
   `arcp_vers=2.7.41`  
   `arcp_min=0.2.4`  
   `router_vers=4.0.2`  
   `router_info={board}`  
   `mf={manufacturer}`  
   `model={model}`

2. `{hostname}._netaudio-cmc._udp.local.` SRV = CMC port. TTL 4500.  
   TXT:  
   `id={16 lowercase hex device_id}`  
   `process={process_id decimal}`  
   `cmcp_vers=1.2.0`  
   `cmcp_min=1.0.0`  
   `server_vers=4.0.2`  
   `channels=0x6000004d`  
   `mf=…` `model=…`  
   then **two zero-length TXT strings**. Keep until a pcap proves they are unused.

Also publish `{hostname}.local` A record. Hostname: one label, ≤31, charset `[A-Za-z0-9-]`.

Do **not** announce `_netaudio-chan`, `_netaudio-dbc`, `_netaudio-bund` when `tx_channels=0`.

**Probe/announce (RFC 6762 §8):** random delay 0–250 ms; **three probes** 250 ms apart (ANY on hostname and service instance names); on conflict, suffix-rename and restart; **two unsolicited announcements** ~1 s apart from **:5353**; then answer queries packing PTR+SRV+TXT+A.

**Querier (subscribe):** bind `iface:0`, `set_multicast_if_v4`, send query **to** `224.0.0.251:5353`, read **unicast** replies on the high port. Do not bind 5353 on the querier.

Phase 2 query `_netaudio-chan`: instance `{tx_channel}@{tx_hostname}._netaudio-chan._udp.local`. TXT of interest: `dbcp1=0x1102`, `fpp={max},{min}`, `enc`/`rate`/`nchan`, SRV port = flows-control (usually 4455).

### 8.10 PTPv1 (IEEE 1588-2002) — parse only

Join `224.0.1.129`. Event 319, general 320.

40-byte common header, BE:

| off | len | field |
| --- | --- | --- |
| 0 | 2 | versionPTP (=1) |
| 2 | 2 | versionNetwork (=1) |
| 4 | 16 | subdomain (default `_DFLT` + NULs) |
| 20 | 1 | messageType (1=event, 2=general) |
| 21 | 1 | source communication technology |
| 22 | 6 | sourceUuid |
| 28 | 2 | sourcePortId |
| 30 | 2 | sequenceId |
| 32 | 1 | control |
| 33 | 1 | reserved / logMeanMessageInterval |
| 34 | 2 | flags |
| 36 | 4 | reserved |

`control`: `0` Sync (UDP 319), `2` Follow_Up (UDP 320). Drop `versionPTP != 1`. Drop PTPv2.

Sync payload @ 40: `originTimestamp` = u32 seconds + i32 nanoseconds. If non-zero, one-step: `t1` is this; skip Follow_Up wait.

Follow_Up payload @ 40: u16 `associatedSequenceId`, u16 reserved, 8-byte `preciseOriginTimestamp` (`t1`).

Need ≥ 48 bytes for Follow_Up. Truncated → drop.

PTPv1 has **no `domainNumber`**. Isolation is the 16-byte subdomain.

---

## 9. Networking (Windows first-class)

### 9.1 Bind rules

Reject in `Settings::validate` **and** in the bind helper: `Ipv4Addr::UNSPECIFIED`, multicast, broadcast → `UnspecifiedAddress`. Loopback allowed for tests. Link-local `169.254/16` allowed.

`Bind::Ip`: that address must exist on an up NIC.  
`Bind::Interface`: `netdev::get_interfaces()`, match in order: IPv4 string, `name`, Windows friendly/description (**case-insensitive**). First non-unspecified unicast IPv4. Else `InterfaceNotFound` / `InterfaceHasNoIpv4`.

Do **not** use `local_ip_address::local_ip()` (picks VPN). If bind is omitted, fail closed (`InvalidSettings`).

`socket2`: set options **before** `bind`. Control sockets: `set_nonblocking(true)` then `tokio::net::UdpSocket::from_std`. Media/clock/mDNS: stay `std`/`socket2` on their threads.

### 9.2 Per-role socket options

**Unicast control (ARC, CMC, info 8700):**

- Windows: `SO_EXCLUSIVEADDRUSE = 1` (raw `setsockopt`; not in socket2), `SO_REUSEADDR` off.
- Unix: no reuse, no reuseport.
- Info only: `set_multicast_if_v4(iface)` so sends to 231/233 leave the Dante NIC. TTL 1. Do not join those groups.
- Do not `connect()`.

**mDNS announcer (`iface:5353`):**

- Windows: `SO_REUSEADDR` on, exclusive-use **off**, bind `iface:5353`.
- Linux/macOS: `SO_REUSEADDR` + `SO_REUSEPORT`, bind `iface:5353`.
- Then `set_multicast_if_v4`, `join_multicast_v4(224.0.0.251, iface)`, `set_multicast_ttl_v4(255)`, loopback **on** (two-instance / fake tests).
- Never bind the group address (Windows fails or receives nothing).
- `WSAEADDRINUSE` (10048) and `WSAEACCES` (10013) → `MdnsPortInUse`.

**PTP 319/320:**

- Bind `iface:319` and `iface:320`. Join `224.0.1.129` on iface. TTL 1.
- **No** `SO_REUSEPORT` (Linux hashes datagrams; Follow_Up would be lost). Prefer exclusive bind. If DVS holds the ports: `PtpBindDenied`, continue with media-driven clock.
- Windows: no admin. Unix `EACCES` → `PtpBindDenied`.

**Media RX:**

- Bind `iface:port` with `port` in `0x3800..=0x397F`.
- Unconnected. `recv_from` + `send_to(keepalive, last_source)`.
- Windows: `SIO_UDP_CONNRESET = FALSE` on every UDP socket we recv on. **Never `connect()`** (ICMP port-unreachable becomes `WSAECONNRESET`).
- `set_recv_buffer_size(2 * 1024 * 1024)`; log actual.

Multicast media RX is **not v1**. When added: bind `iface:port` + join; do **not** bind 239.x on Windows.

### 9.3 Device ID

If `Settings.device_id` is `Some`, use it. Else after resolving the NIC:

- MAC non-zero: `[0x00, 0x00, mac0, mac1, mac2, mac3, mac4, mac5]`
- else: `[0x00, 0x00, ip0, ip1, ip2, ip3, pid_hi, pid_lo]`

Two processes on one IP must set distinct `process_id` / `device_id`.

### 9.4 Firewall (document in README; return live list from `bound_ports`)

RX v1 inbound UDP: 4440, 8800, 8700 (or ALT range except +2), 5353, ephemeral/0x3800–0x397F media. Phase 3: 319, 320.

Outbound: TX `:4455`, `224.0.0.231:8702`, `224.0.0.233:8708`, `224.0.0.251:5353`, `224.0.1.129:319/320`.

Inbound 4455 is TX/DBC only. Prefer a Windows **application** allow-rule; leave built-in mDNS (UDP-In) enabled. Linux: `CAP_NET_BIND_SERVICE` for 319/320.

ARC is UDP **4440**.

---

## 10. Overlay clock

### 10.1 Types

| Name | Rust | Role |
| --- | --- | --- |
| `FineTime` | `u64` | wrapping nanoseconds (local **and** overlay) |
| `FineTimeDiff` | `i64` | wrapping deltas and `shift` |
| `freq_scale` | `f64` | fractional rate; `1e-6` = 1 ppm |
| `SampleIndex` | `u64` | wrapping sample count |

Never Unix epoch. Never `SystemTime`.

### 10.2 Local time

One `Instant` origin at `Device::start`:

```
local_ns() = origin.elapsed().as_nanos() as u64
```

Windows: this is QPC.

### 10.3 Formula (wrapping)

```
elapsed = (local_ns as i64).wrapping_sub(last_sync as i64)
corr    = (elapsed as f64 * freq_scale) as i64   // trunc toward 0
overlay = (local_ns as i64).wrapping_add(shift).wrapping_add(corr) as u64
```

Do **not** subtract `last_sync` in `u64`. After every accepted observation, set `last_sync = t2` and rewrite `shift` so `overlay(t2) = new_overlay_at_t2` (elapsed 0).

### 10.4 PTPv1 listen-only servo

On Sync (`control=0`, UDP 319): store `{sequenceId, sourceUuid, t2 = local_ns()}` immediately. If `originTimestamp != 0`, one-step: `t1` from it.

On Follow_Up (`control=2`, UDP 320): match uuid + `associatedSequenceId`. `t1` from `preciseOriginTimestamp` (u32 seconds + i32 nanos → ns). Expire unmatched Sync after 2 s. Filter subdomain.

**Acquire (first sample):**

```
last_sync = t2
last_master = t1
shift = (t1 as i64).wrapping_sub(t2 as i64)
freq_scale = 0.0
acquiring = true
```

**Later:**

```
local_dt  = (t2 as i64).wrapping_sub(last_sync as i64)
master_dt = (t1 as i64).wrapping_sub(last_master as i64)
```

Reject if `local_dt <= 0` or `master_dt <= 0`, or `local_dt` not in `[200ms, 4s]`, or `|master_dt - local_dt| > 50ms`.

```
raw_scale = clamp((master_dt as f64 / local_dt as f64) - 1.0, -500e-6, +500e-6)
freq_scale = 0.875 * freq_scale + 0.125 * raw_scale    // EMA, PTP native ~1 Hz
```

**Outlier:** if applying the observation would move `overlay_now()` by **> 200 ms**, drop it.

**Slew:** `pred = overlay(t2)` using the **old** snapshot. `error = (t1 as i64).wrapping_sub(pred as i64)`. `max_step = acquiring ? 1_000_000 : 50_000` ns. `applied = clamp(error, ±max_step)`.

```
new_overlay_at_t2 = pred.wrapping_add_signed(applied)
last_sync = t2
last_master = t1
shift = (new_overlay_at_t2 as i64).wrapping_sub(t2 as i64)
```

Lock: `acquiring = false` after **4** consecutive `|error| < 100 µs`. Any reject or `|error| > 1 ms` → acquiring again.

No PI. No Delay_Req. One-way path delay lives in `shift` and is absorbed by `rx_latency ≥ 4 ms`.

### 10.5 Media-driven fallback

Same servo. `t1` from the 9-byte header:

```
t1 = seconds * 1_000_000_000 + index * 1_000_000_000 / rate   // u128 mid
t2 = local_ns()
```

Decimate to **≤ 8 Hz** (one observation per 125 ms). Prefer PTP when `lock_streak ≥ 4`; ignore media for the servo while PTP-locked (media still writes the ring). On source switch, **do not reset** `shift`/`freq_scale`; feed new pairs into the same slew.

If no PTP lock and no media packets for 1 s: `ClockStatus::Unlocked`. Overlay remains readable; do not step it.

### 10.6 Seqlock snapshot

`ClockOverlay` is `Copy`. Writer: bump seq odd, store `last_sync`, `shift`, `freq_scale` bits, bump seq even (`Release`). Reader: load even seq, `Acquire` fields, reload; retry if odd or changed. Audio path: no `RwLock`.

### 10.7 Sample conversion

```
ns_to_samples(ns) = (ns as u128 * rate as u128 / 1_000_000_000) as u64
samples_to_ns(s)  = (s as u128 * 1_000_000_000 / rate as u128) as u64
```

`try_read` due time:

```
due = ns_to_samples(overlay_now())
read_pos = due.wrapping_sub(rx_latency_samples)
```

`AudioFrameMut.media_time.sample_index = read_pos` (first sample). `ns = samples_to_ns(read_pos)`. Not `Instant::now()`.

---

## 11. Media RX and PCM

### 11.1 Ring

- **Internal:** planar, one lane per **device** RX channel, indexed by wrapping `SampleIndex`.
- Length: power of two, `>= max(2048, 4 * rx_latency_samples)`.
- No writer-side silence lookahead (do not pre-zero future slots).

Write packet at `ts = sample_index`, `n = FPP` frames, map flow slot `c` → device channel `d`:

- `pos = ts.wrapping_add(k)`
- if `pos` already consumed (`wrapped_diff(read_pos, pos) > 0`) → drop frame, count `late_frames`
- if `wrapped_diff(pos, read_pos) >= ring_len` → drop packet, count `overrun`
- else store promoted `i32`

Reorder is OK. Writes are idempotent at a given `pos`.

### 11.2 Read policy

- **No subscriptions:** `try_read` returns `0`. Do not write zeros.
- **At least one patched channel:** copy a full-width interleaved block. Unpatched channels in that block are 0. Due but missing samples (loss/late): **0**, count `underrun_samples`. Do not block waiting for reorder.
- After `rx_latency + 50 ms` with no packet on any patched flow: pause (`try_read` returns 0) until traffic resumes. Next `media_time` comes from new packet timestamps (no step of overlay).

Block size: `min(64, fpp_max, rx_latency_samples / 2)` frames per `try_read` call; caller may request fewer via `dst.samples.len() / channels`.

### 11.3 Channel mapping

`0x0100` `channel_ids[n]` are 1-based **TX** ids. Local map is built at subscribe: flow slot `i` → local RX index. Max 8 channels per flow, max 32 RX flows.

### 11.4 Left-justified `i32`

Unused LSBs are zero. Wire values are integer PCM shifted into the high bits.

| Bits | Wire BE | `Sample` |
| --- | --- | --- |
| 16 | `b0 b1` | `i32::from(i16::from_be_bytes([b0,b1])) << 16` |
| 24 | `b0 b1 b2` | `((b0 as i8 as i32) << 24) \| ((b1 as i32) << 16) \| ((b2 as i32) << 8)` |
| 32 | `b0..b3` | `i32::from_be_bytes` |

Examples: `0x7FFF` → `0x7FFF_0000`; 24-bit `0x7FFFFF` → `0x7FFF_FF00`.

Malformed length: drop packet, no panic; keep whole frames only.

Public API is **interleaved**. Convert planar → interleaved in `try_read`.

### 11.5 Media thread

`polling` 3 on all flow sockets + command wakeup. Commands from control: add/remove socket, remap (lock-free queue or self-pipe). This thread stays in the media plane (UDP + ring + keepalive).

Keepalive on this thread.

---

## 12. Subscribe orchestration (Phase 2)

On ARC `0x3010` (control/tokio):

1. Parse records. Persist desired map `local_rx → (tx_hostname, tx_channel_name) | None`.
2. ACK empty OK immediately (do not block DC on mDNS).
3. For each new sub: query mDNS `_netaudio-chan` `{ch}@{host}` (and `_netaudio-arc` for A/ARC port if needed). Read `fpp`, `enc`, `rate`, SRV port (flows-control).
4. If `rate != Settings.sample_rate`, set status `0x00000001` / log; do not send `0x0100`.
5. Bind media UDP in `0x3800..=0x397F`. Send `0x0100` to TX flows-control. Pass socket to media thread. Status `0x00000008` then `0x01010009` after first packet.
6. Unsubscribe: `0x0101`, media-thread remove, status 0.

Do not resolve mDNS inside `protocol` codecs.

---

## 13. Work items (implement in this order)

Each item is one LLM session. Do not skip tests. Do not implement TX production code in v1.

### W0 — Settings / Error / BeBuf

**Files:** `src/lib.rs`, `src/device/{mod,settings,error}.rs`, `src/protocol/buf.rs`, `tests/settings.rs`  
**Do:** types in §6; `validate`; `Sample`; empty `clock`/`media`/`net` modules. `pub(crate) mod protocol` etc.  
**Tests:** reject 3.9 ms; reject 32-byte name and names with `.` or space; reject 32000 Hz; reject `tx_channels != 0`; accept 4 ms / 48000 / B24.  
**Done:** `cargo test` green; no sockets.

### W1 — Protocol codecs (no I/O)

Split:

| ID | Files | Tests / done |
| --- | --- | --- |
| W1a | `protocol/req_resp.rs` | 10-byte header roundtrip |
| W1b | `protocol/media.rs`, `pcm.rs` | 9-byte header; 16/24/32 table §11.4 |
| W1c | `protocol/arc.rs` | `0x1000` 36 B, `0x1003`, empty pages, `0x3000` one page, `0x3010` sub/unsub, `0x3300` golden hex, pagination `0x8112` |
| W1d | `protocol/cmc.rs` | 22-byte advertisement |
| W1e | `protocol/flows_control.rs` | `0x0100` layout + golden; `0x0101`/`0x0102`; errors |
| W1f | `protocol/info_mcast.rs` | 32-byte header; board `[0xBB]=0x1F` |
| W1g | `protocol/mdns.rs` | PTR/SRV/TXT/A; two empty CMC TXT |
| W1h | `protocol/ptp_v1.rs` | Sync/Follow_Up; wrong version dropped |

**Done:** `protocol` has no `std::net` I/O. Fixtures under `tests/fixtures/` with provenance comments.

Golden must-pass hex:

```
0x3300 req:  2729000a033c33000000
0x3300 resp: 27290012033c330000013800397f398039ff
keepalive:   1337
```

### W2 — Bind + multicast + BoundPorts

**Files:** `src/net/{mod,udp,iface,ports,windows}.rs`, `tests/bind.rs`  
**Do:** §9 bind helper; reject unspecified; `netdev` resolve; multicast join with iface; Windows exclusive-use / `SIO_UDP_CONNRESET`.  
**Tests:** unspecified rejected; interface name resolution on loopback; multicast join arguments.  
**Done:** no servers yet.

### W3 — Appear in DC

| ID | Do | Done |
| --- | --- | --- |
| W3a | mDNS announcer thread, probe+announce ARC+CMC | encode+bind tests; manual: `dns-sd`/`avahi` sees name |
| W3b | Tokio CMC `0x1001` | fixture request → advertisement |
| W3c | Tokio ARC opcode set | UDP client in test: `0x1000` returns `rx_count` |

**Manual acceptance:** DC shows name, RX count, IP. CI does not require DC.

### W4 — Info multicast

**Files:** `device/info_mcast.rs`  
**Do:** board+product at start; 1 Hz heartbeat; `0xBB=0x1F`; answer `0x61`/`0xC1`/`0x13` if received.  
**Tests:** destinations 8702/8708; IF; seqnum.  
**Done:** packets fire on interval.

### W5 — Overlay + ring (no subscribe)

| ID | Files | Done |
| --- | --- | --- |
| W5a | `clock/overlay.rs` | wrap, slew caps, 200 ms drop, EMA 100 ppm synthetic |
| W5b | `media/ring.rs` | reorder 2 packets; late drop; unpatched `try_read==0`; patched hole → zeros |

### W6 — Subscribe + media RX

Put test doubles in `src/testutil/` behind `#[cfg(test)]` **in this work**, not later.

| Double | Behavior |
| --- | --- |
| `FakeFlowsControl` | Bind `127.0.0.1:alt+2`. `0x0100` → handle; rate mismatch → `0x0301`; `0x0101` stop; expire after 4 s without keepalive |
| `FakeMediaSource` | Unicast 9-byte header + interleaved BE PCM ramp. Optional: wait for first `1337` |
| `SubscribeOverride` | Skip mDNS in unit tests: `{ipv4, flows_port}` |

| ID | Do |
| --- | --- |
| W6a | `device/subscribe.rs` state machine |
| W6b | mDNS querier (legacy unicast) |
| W6c | media thread + `0x0100` client + `try_read` vs fakes |

**Done:** known ramp through `try_read`; unsub stops packets; keepalive seen by fake TX.

### W7 — PTPv1 listen-only

**Files:** `clock/ptp.rs`  
**Do:** bind 319/320; servo §10.4; `PtpBindDenied` non-fatal.  
**Tests:** fixture Sync+Follow_Up → `PtpLocked`; wrong subdomain ignored.  
**Done:** unit tests. 1-hour soak is **manual**.

### W8 — Loopback integration

**Files:** `tests/loopback_rx.rs`  
**Do:** one `Device` on `127.0.0.1` + fakes. Inject subscribe (internal or crafted `0x3010`). Assert ramp, then unsub.  
**Done:** CI green without hardware and without #8.

### W9 — Docs

**Files:** `README.md` ports/firewall/`CAP_NET_BIND_SERVICE`/legal; this plan stays canonical.  
**Done:** `cargo doc` builds; `bound_ports` listed.

**v1 ship bar:** W0–W9 + manual DC appearance + optional hardware RX soak.

---

## 14. Phase / issue acceptance matrix

| Phase | Issues | Automated | Manual |
| --- | --- | --- | --- |
| 0 | #2 | this document merged | — |
| 1 | #3 partial, #5, #6 | W0–W4 | DC device list |
| 2 | #3 rest, #7, #4 media clock | W5–W6, W8 | DC patch → PCM |
| 3 | #4 PTP | W7 | DC clock OK; 1 h RX |
| 4 | #8 | later | DC TX patch |
| 5 | #9 | later | play on WASAPI/CoreAudio/ALSA |
| 6 | #10 | later | DAW loads ASIO/VST3 |

Same public API compiles on Windows 10/11, macOS, Linux. Overlay clock runs in-process. OS time stays under the host’s control.

---

## 15. Post-v1 (specify now, implement later)

### 15.1 Phase 4 — TX (#8)

- Allow `tx_channels >= 1`. `tx_latency >= 4 ms`.
- `try_write(&self, src: AudioFrameRef<'_>) -> Result<usize, Error>` at overlay **+ tx_latency**.
- Listen flows-control **server** 4455 (`alt+2`): `0x0100`/`0x0102`/`0x0101`. Handle 6 bytes. Errors `0x0103`/`0x0315`/`0x0301`. Max 32 TX flows, max 8 ch/flow, FPP clamp to MTU, advertised max 32.
- Media header byte 0 = `0x02`. Unicast only. Keepalive timeout 4 s.
- ARC `0x2000`/`0x2010`/`0x2200` filled. Announce `_netaudio-chan`. Optionally `_netaudio-dbc`.
- **Do not:** multicast TX, PTP leader, AES67, DDM, `tx_latency < 4 ms`, cpal in core.

### 15.2 Phase 5 — cpal (#9)

- Feature `cpal` on this crate; optional dep `cpal` 0.18. Core modules must not mention cpal types.
- Adapter: `try_read`/`try_write` + resample. **Dante overlay is always clock master.** Matching nominal rate still needs adaptive SRC (two oscillators).
- Missing overlay lock: do not start the stream (or silence **in the adapter**, not in core).
- cpal ASIO host is **not** #10.

### 15.3 Phase 6 — user-space ASIO / VST3 (#10)

Separate crates/repos: `netaudio-asio` (`cdylib` IASIO), `netaudio-vst3`. WDM/kernel/WHQL **out of scope**. Do not vendor Steinberg SDK in this MIT tree. Host callback is another slave clock → SRC. PTP leader still out of scope.

---

## 16. Risks and mitigations

| ID | Risk | Mitigation |
| --- | --- | --- |
| R1 | mDNS 5353 coexistence (Bonjour / Windows DNS Client) | reuse flags §9.2; `MdnsPortInUse`; no high-port announce |
| R2 | DC sends undocumented ARC opcodes | log + no reply; add stubs from pcap; known: `0x1100`/`0x1102`/`0x2320`/`0x3300` |
| R3 | PTP privilege on Unix | `PtpBindDenied`; media-driven RX still works; document `setcap` |
| R4 | Windows default multicast IF | always `IP_MULTICAST_IF` + join with iface IPv4 |
| R5 | GPL contamination | original MIT source; hex fixtures with provenance |
| R6 | DC clock domain mismatch | status `0x001B` vs `0x3300` ports; PTP lock + heartbeat `0x8001` |
| R7 | hostname > 31 | `validate` rejects |
| R8 | Dual consumers of the ring | pull XOR wakeup; single consumer |
| R9 | WinSock `WSAECONNRESET` | unconnected media + `SIO_UDP_CONNRESET` |
| R10 | Info flood | `board[0xBB]=0x1F` |

---

## 17. Implementation checklist (copy into a PR)

- [ ] Original MIT source and comments; crate deps as in §4
- [ ] `edition = "2024"`; tokio features not expanded to `full`
- [ ] `protocol` / `net` / `media` not public
- [ ] No bind of `0.0.0.0`
- [ ] Latency floor 4 ms enforced
- [ ] `tx_channels == 0` in v1
- [ ] Left-justified PCM tests
- [ ] `0x3300` golden hex
- [ ] `0x0802` socket magic (not `0x8002`)
- [ ] String offsets from packet start
- [ ] Pagination 1-based + `0x8112`
- [ ] Two empty CMC TXT records
- [ ] Info `0xBB = 0x1F`
- [ ] Keepalive `13 37` / 250 ms / last source
- [ ] Overlay i64 wrapping + seqlock
- [ ] PTP listen-only, subdomain `_DFLT`
- [ ] `try_read` 0 when unpatched
- [ ] Windows exclusive-use on control; reuse on 5353; no connect on media
- [ ] README firewall + `CAP_NET_BIND_SERVICE` + disclaimer
- [ ] Loopback test with fake TX

---

## 18. Research sources (read; keep out of the tree)

- Original RFC (git `7613c55`: `docs/ISSUE-001-rx-crate-plan.md`) — product intent
- [network-audio-controller](https://github.com/chris-ritsen/network-audio-controller) (Unlicense) and its wiki Technical-details — captures and ARC command IDs
- IEEE 1588-2002 — PTPv1 header
- RFC 6762 / 6763 — mDNS / DNS-SD

When a capture from Dante Controller + hardware disagrees with this plan, **the capture wins**, and this file is updated in the same change.
