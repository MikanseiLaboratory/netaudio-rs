---
title: protocol 層 — ARC / CMC / flows-control / メディアヘッダの ser/de
labels: enhancement
github: 3
---

# protocol 層 — ARC / CMC / flows-control / メディアヘッダの ser/de

`netaudio::protocol` は **パケットの読み書きだけ** を持つ。ユニットテストと pcap fixtures で閉じる。

親: [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2)。次: [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6)（制御面 I/O）、[#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7)（メディア I/O）。

## Inferno の対応ファイル

`inferno_aoip/src/protocol/`（`dev` / `master` / `transmit` で同じ構成）:

| ファイル | 担当 |
| --- | --- |
| `proto_arc.rs` | ARC（ルーティング、チャンネル名、フロー数）UDP 4440 |
| `proto_cmc.rs` | CMC（デバイス制御）UDP 8800 |
| `flows_control.rs` | TX への subscribe 要求 UDP 4455 |
| `mcast.rs` | info / heartbeat マルチキャスト |
| `req_resp.rs` | 要求・応答の共通枠 |

メディア 9 バイトヘッダは `device_server/flows_rx.rs`（`dev`）および `master` の `flows_rx.rs`。

`binary_packets_refactor` ブランチはパケット整理の差分確認用。フィールド意味が変わっていないかを見る。

## v1 で揃えるもの

- mDNS TXT / サービス型 `_netaudio-arc._udp.local` / `_netaudio-cmc._udp.local`
- ARC / CMC の **DC に出る最小セット**（名前、IP、RX チャンネル数、サンプルレート）
- flows-control の unicast subscribe / unsubscribe
- info multicast の heartbeat 骨格
- メディアヘッダ: 秒 + サンプル位置 + ペイロード長からチャンネル数を検証
- PCM: BE 16/24/32-bit integer → 内部 `i32`

## 受け入れ

- Inferno / DVS / 実機の pcap を fixtures にして encode/decode が往復する
- `protocol` モジュールはパケットの ser/de だけ（I/O は [#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) / [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6)）
- ser/de は自前（または `byteorder`）

## Later（この Issue の外）

チャンネル名変更の完全な ARC、ピーク／遅延の info パケット（`latency_reporting`）、TX multicast 広告。
