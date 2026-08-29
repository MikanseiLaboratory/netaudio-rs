---
title: cpal feature — 既存 OS デバイスへの再生とキャプチャ
labels: enhancement
---

# cpal feature — 既存 OS デバイスへの再生とキャプチャ

Phase 5。`netaudio` 本体は OS 音声 API を知らない。feature `cpal`（またはクレート `netaudio-cpal`）が `AudioBlock` を既存デバイスのコールバックへつなぐ。

親: #001。依存: #006（RX）、将来 #007（TX）。

## 成果物

受信 PCM を **すでに存在する** 出力デバイスへ出す。将来は既存入力を TX へ。

接続先: WASAPI / CoreAudio / ALSA、環境により ASIO **ホスト**（cpal の ASIO は弱いので、足りなければ #009 で `asio-sys`）。

## クロック

- 再生: Dante メディア時計がマスター。デバイスがマスターのときは **リサンプル** してデバイスコールバックへ
- TX: デバイス時計がマスターならリサンプルしてネットワークへ。4 ms はジッタ余裕

リサンプル実装の選定はこの Issue で決める。

## 受け入れ

- Windows で WASAPI 出力デバイスから Dante RX が聞こえる
- macOS / Linux でも同じ feature がビルドできる
- `netaudio` の default feature は cpal なしのまま
