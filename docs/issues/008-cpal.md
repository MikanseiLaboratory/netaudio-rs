---
title: cpal feature — 既存 OS デバイスへの再生とキャプチャ
labels: enhancement
github: 9
---

# cpal feature — 既存 OS デバイスへの再生とキャプチャ

Phase 5。feature `cpal`（またはクレート `netaudio-cpal`）が `AudioBlock` を既存デバイスのコールバックへつなぐ。`netaudio` の default は PCM API。

親: [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2)。依存: [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7)（RX）、将来 [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8)（TX）。

## 成果物

受信 PCM を **すでに存在する** 出力デバイスへ出す。将来は既存入力を TX へ。

接続先: WASAPI / CoreAudio / ALSA、環境により ASIO **ホスト**（cpal の ASIO が足りなければ [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) で `asio-sys`）。

## クロック

- 再生: Dante メディア時計がマスター。デバイスがマスターのときは **リサンプル** してデバイスコールバックへ
- TX: デバイス時計がマスターならリサンプルしてネットワークへ。4 ms はジッタ余裕

リサンプル実装の選定はこの Issue で決める。

## 受け入れ

- Windows で WASAPI 出力デバイスから Dante RX が聞こえる
- macOS / Linux でも同じ feature がビルドできる
- default feature は PCM API のまま。`cpal` は feature フラグ
