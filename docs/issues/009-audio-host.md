---
title: オーディオホスト — ユーザー空間 ASIO / VST3
labels: enhancement
github: 10
---

# オーディオホスト — ユーザー空間 ASIO / VST3

Phase 6。調査と設計。実装範囲は後続 Issue で切る。

親: [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2)。Inferno [#3](https://github.com/teodly/inferno/issues/3) の整理を踏まえた成果物:

- **ユーザー空間 ASIO DLL** — 署名不要で DAW から見える（Windows）
- **VST3** — クロスプラットフォーム。DAW の既存 IF がマスターなので **リサンプル必須**。インストールが楽
- Linux 参考: Inferno の `alsa_pcm_inferno` / `jack-compat`

## いま固定する方針

1. v1〜Phase 5 は PCM API と cpal ホストまで
2. DAW 直結が必要になったら、先に VST3（クロスプラットフォーム）かユーザー空間 ASIO DLL（Windows）を選ぶ
3. Inferno の ALSA プラグインは Linux の参考実装

## 受け入れ（将来）

後続 Issue で実装範囲と署名／インストーラを切る。
