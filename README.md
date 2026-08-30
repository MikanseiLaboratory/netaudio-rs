# netaudio-rs

Rust 向けの Dante 互換 Audio-over-IP ライブラリです。Audinate とは独立した研究開発です。

`netaudio` は受信を先に実装したライブラリです。プロセスが Dante 機器として見え、Dante Controller からのパッチを受け、PCM をアプリケーションへ渡します。クロックはプロセス内の overlay（`Instant` / QPC）です。Windows を第一級の対象とし、macOS と Linux でも同じ API を使えます。

プロトコルのパケットレイアウトと、制御 / メディア / クロックの三面構成は、公開キャプチャと本リポジトリの仕様から起こした **MIT の独自実装** です。

Dante は Audinate Pty Ltd の商標です。

## 現状

プライベート R&D です。製品計画は GitHub Issues にあります。実装仕様（技術選定、プロトコル、公開 API、作業順）は [`docs/IMPLEMENTATION-PLAN.md`](docs/IMPLEMENTATION-PLAN.md) です。

| GitHub | 内容 |
| --- | --- |
| [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2) | トラッキング RFC — Windows 第一級の受信クレート |
| [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3) | プロトコルコーデック（ARC / CMC / flows-control / メディアヘッダ） |
| [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) | プロセス内 overlay 時計（PTPv1 待ち受け + メディア時刻） |
| [#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) | インタフェース固定ソケット、マルチキャスト、mDNS 5353、PTP ポート |
| [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) | 制御面 — Dante Controller に表示する |
| [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7) | メディア RX — 購読、リングバッファ、PCM 読み出し |
| [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8) | TX — ユニキャスト、`tx_latency` ≥ 4 ms |
| [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) | `cpal` 機能 — 既存の OS デバイス |
| [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) | オーディオホスト — ユーザー空間 ASIO / VST3 |

v1 は Phase 3 です。制御面、メディア RX、PTPv1 にロックする overlay 時計までを含みます。

## v1

| 項目 | v1 |
| --- | --- |
| 形態 | ライブラリ（`netaudio`）。制御・メディア・クロックはプロセス内で動作します |
| OS | Windows / macOS / Linux |
| 音声 I/O | PCM は **`Device::try_read`** で取得します。任意で `set_rx_wakeup` を使えます。OS 再生は feature `cpal`（[#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) の再生スライス）。キャプチャと adaptive SRC は未着手です |
| レイテンシ | 設定可能です。下限は **4 ms**（DVS クラスのソフトウェアクロック）です |
| クロック | プロセス内 overlay です。PTPv1 を待ち受け、メディアパケットの時刻も使います |

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

`Device::start` は、呼び出し側の tokio ランタイム上で動きます。`try_read` は未パッチまたは未到来のとき `0` を返し、呼び出し側バッファの内容はそのままです。

## 受信 CLI（example）

Windows / macOS / Linux 共通です。clap で引数を取り、Dante 機器として起動します。既定では OS の **default 出力**（Windows は WASAPI）で再生し、1 秒ごとにピークを表示します。`--no-play` でピークのみです。

```text
cargo run --example rx --features cpal -- ifaces
cargo run --example rx --features cpal -- listen --bind 192.168.1.10 --name studio-rx --rx-channels 2
cargo run --example rx --features cpal -- listen --bind 192.168.1.10 --no-play
```

`--bind` には IPv4 かインタフェース名（Windows では表示名）を渡します。example は feature `cpal` が必要です。`listen --help` でビット深度・サンプルレート・レイテンシ・`alt-port`・`--play` / `--no-play` を確認できます。Ctrl+C で停止します。

ソースは [`examples/rx.rs`](examples/rx.rs) です。

## ポートとファイアウォール

すべての UDP ソケットは、設定した **ユニキャスト IPv4** に bind します。そのインタフェースで次のポートを開けてください。

| 面 | UDP | 内容 |
| --- | --- | --- |
| ARC | 4440（`alt_port+0`） | Dante Controller の制御 |
| CMC | 8800（`alt_port+1`） | 機器アドバタイズ |
| Info | 8700（`alt_port+3`） | bind。送信先は `224.0.0.231:8702` と `224.0.0.233:8708` |
| メディア RX | `0x3800..=0x397F` | TX からのユニキャスト。keepalive `[0x13,0x37]` を 250 ms 間隔 |
| mDNS | 5353 | アナウンスの送信元は 5353 です。`224.0.0.251` に参加します |
| PTP | 319 / 320 | 待ち受け。グループ `224.0.1.129`。ポート番号は固定で、`alt_port` は ARC / CMC / flows / info だけをずらします |

v1 の flows-control **4455**（`alt_port+2`）はクライアントです。TX へ `0x0100` を送ります。

使うマルチキャストグループは `224.0.0.251`（mDNS）、`224.0.0.231` / `224.0.0.233`（info / heartbeat）、`224.0.1.129`（PTPv1）です。

### Linux の PTP bind

UDP 319/320 には `CAP_NET_BIND_SERVICE`（または root）が必要なことがあります。未設定のときはメディア駆動の overlay で起動を続けます（`Error::PtpBindDenied`）。例:

```
sudo setcap cap_net_bind_service=+ep /path/to/your/binary
```

### Windows

制御ソケットは exclusive-address-use、mDNS は reuse、メディアソケットは未接続のまま使います（`SIO_UDP_CONNRESET` を切る）。送信側リセット後も受信を続けられます。

## 後続

- TX チャネル（[#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8)）
- `cpal` 機能の残り: キャプチャと adaptive SRC（[#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9)）。再生は example と `netaudio::play` にあります
- ユーザー空間 ASIO DLL / VST3（[#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10)）

## ライセンス

MIT
