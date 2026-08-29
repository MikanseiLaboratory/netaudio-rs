---
title: メディア RX — subscribe、リングバッファ、AudioBlock API
labels: enhancement
---

# メディア RX — subscribe、リングバッファ、AudioBlock API

Phase 2。Dante Controller からこのデバイスの RX にパッチすると、アプリのコールバックに PCM が届く。

親: #001。依存: #002, #004, #005。時計は最初メディアパケット駆動（#003 の優先 2）でよい。

## Inferno の対応

- `dev`: `device_server/flows_rx.rs`, `channels_subscriber.rs`, `ring_buffer.rs`
- `master`: `flows_rx.rs`, `cirb`（Clock-Indexed Ring-Buffer）
- keepalive `0x13 0x37`、250 ms

受信スレッドと Tokio 制御面は分ける。Inferno が mio + 専用スレッドにしている理由（リアルタイム、キュー滞留）は正しい。v1 は `thread + nonblocking UDP`。

## API（実装前の契約）

```rust
let device = Device::start(Settings {
    name: "Mikansei RX".into(),
    bind: Bind::Ip(local_v4),
    rx_channels: 8,
    sample_rate: 48_000,
    rx_latency: Duration::from_millis(4), // 下限 4ms
    ..Default::default()
}).await?;

device.set_rx_handler(|block: AudioBlock| {
    // block.media_time  : メディアクロック上の先頭サンプル
    // block.sample_rate
    // block.channels    : planar または interleaved、Sample = i32
});
```

必須設定: `BIND_IP`（IF 名でも可）、`NAME`、`RX_CHANNELS`、`SAMPLE_RATE`、`RX_LATENCY`。

## 動き

- DC がパッチ → flows-control で相手 TX に subscribe → 向こうから UDP が来る
- パケット時刻でリングバッファに書く。アプリは「今のメディア時刻 − latency」から読む
- multicast flow は #004 の IF 指定 join（v1 主経路は unicast。mcast は可能なら足す）
- 未パッチではブロックしない。無音の捏造は任意（Inferno2pipe はフロー無しで時刻が止まる）

PCM は BE integer → 内部 i32。ホストエンディアンでアプリへ。

## 受け入れ

- DC からパッチするとコールバックに PCM が届く
- Windows / macOS / Linux で同じ API
- loopback 2 インスタンス（ポート分離）の自動テスト
- 実機 1 台以上で確認（可能な環境）
