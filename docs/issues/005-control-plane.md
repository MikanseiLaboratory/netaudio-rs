---
title: 制御面 — mDNS + ARC/CMC で Dante Controller に出す
labels: enhancement
---

# 制御面 — mDNS + ARC/CMC で Dante Controller に出す

Phase 1。プロトコル（#002）とソケット（#004）を使い、プロセスを Dante デバイスとして広告する。

親: #001。Inferno `dev` の `device_server/{mdns_server,arc_server,cmc_server,info_mcast_server,flows_control_server}.rs`。

## v1 の最小セット

`Device::start(Settings { name, bind, rx_channels, sample_rate, .. })` のあと:

- `_netaudio-arc._udp.local` / `_netaudio-cmc._udp.local` を指定 IF で広告
- ARC で RX チャンネル数・名前・IP が見える
- CMC でデバイスとして認識される
- info multicast の heartbeat 骨格（時計は「なし」でも広告は出す）

Device ID は未指定なら IP + process から安定生成（Inferno と同趣旨。状態キーになる）。

v1 は単一インスタンス、IPv4、TX チャンネル 0。

## 受け入れ

- Dante Controller のデバイス一覧に `NAME` が出る（Windows 上の DC から、Windows / Linux / macOS 上の本クレート）
- チャンネル数が設定どおり
- 外部デーモンなし

## Later（この Issue の外）

チャンネル名の DC からの変更、ピーク／遅延レポート（`latency_reporting`）、ALT_PORT、DDM。
