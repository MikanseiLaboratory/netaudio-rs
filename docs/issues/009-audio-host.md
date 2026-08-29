---
title: オーディオホスト — ユーザー空間 ASIO / VST3（Later）
labels: enhancement
---

# オーディオホスト — ユーザー空間 ASIO / VST3（Later）

Phase 6。#001 の実装範囲の外。調査と設計だけ先に残す。

Inferno [#3](https://github.com/teodly/inferno/issues/3) の整理:

- Linux は ALSA / PipeWire でユーザー空間仮想デバイスが書ける（`alsa_pcm_inferno` / `jack-compat`）
- Windows の WDM は署名が要る。本リポジトリの対象外
- ユーザー空間 ASIO DLL は署名不要で DAW から見える（現実的）
- **VST3** はクロスプラットフォーム。DAW の既存 IF がマスターなので **リサンプル必須**。インストールが楽

## この Issue でやること（今）

実装しない。方針だけ固定する:

1. v1〜Phase 5 は PCM API と cpal ホストまで
2. DAW 直結が必要になったら、先に VST3（クロスプラットフォーム）かユーザー空間 ASIO DLL（Windows）を選ぶ
3. Inferno の ALSA プラグインは Linux 専用の参考実装

## 受け入れ（将来）

別 Issue で実装範囲と署名／インストーラを切る。
