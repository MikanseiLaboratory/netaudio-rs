---
title: プロセス内 overlay 時計 — PTPv1 listen-only とメディアパケット時刻
labels: enhancement
github: 4
---

# プロセス内 overlay 時計 — PTPv1 listen-only とメディアパケット時刻

Windows 対応の本体。Inferno 作者が [#3](https://github.com/teodly/inferno/issues/3) / [#7](https://github.com/teodly/inferno/issues/7) で挙げた listen-only PTPv1 とメディアパケット時刻を、プロセス内 overlay として実装する。

親: [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2)。Phase 2 ではメディアパケット駆動で先行する。Phase 3 で PTP にロックする。

## モデル

ホストのシステム時刻（NTP）はそのまま。プロセスが持つ overlay だけを進める。

```
overlay_ns = local_ns + shift + (local_ns - last_sync) * freq_scale
```

`local_ns` は `std::time::Instant`（Windows では QPC）。usrvclock の **式** を借り、実装は overlay。

Inferno `dev` の `media_clock.rs` と `transmit` ブランチの同名ファイルが参照。

## v1 の時計ソース（優先順）

| 優先 | 方式 | 用途 | 精度 |
| --- | --- | --- | --- |
| 1 | プロセス内 PTPv1 listen-only（ソフトウェアタイムスタンプ） | Controller に時計ありと見せる、TX 将来、無音生成 | 4 ms バジェットで足りる |
| 2 | 受信メディアパケットのタイムスタンプで駆動 | PTP 待ちの RX | フローが切れると時刻が止まる（Inferno2pipe と同じ） |

Later: PTPv2 / AES67、ハードウェアタイムスタンプ、PTP leader。

## ポート

- **Windows**: UDP 319/320 を管理者なしで bind できる前提。失敗したら明示エラー
- **Linux / macOS**: privileged。`CAP_NET_BIND_SERVICE` / root をドキュメント。ライブラリは bind 失敗を返す。PTP はプロセス内

## 遅延

`rx_latency` / `tx_latency` はナノ秒。デフォルト 10 ms、最小 **4_000_000 ns**。ソフトウェア RX タイムスタンプの揺らぎを playout 遅延で吸収する（DVS と同じ理由）。

メディアスレッド優先度は best-effort。Windows は `THREAD_PRIORITY_TIME_CRITICAL` 相当を試し、失敗しても動作する。

## 受け入れ

- overlay 単体のユニットテスト（shift / freq_scale の合成）
- メディアパケット駆動で Phase 2 の RX が動く
- PTPv1 listen-only で DC のクロック表示が安定する（Phase 3）
- 1 時間オーダーの連続 RX で `rx_latency` 4 ms が続く（負荷は別測）
- ホストの NTP オフセットがクレート起動前後で一致する
