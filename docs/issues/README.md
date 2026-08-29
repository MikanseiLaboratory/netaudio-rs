# 計画

GitHub Issues とこのディレクトリは同じ本文。ファイル `001`–`009` は [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2)–[#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10)。

| ファイル | GitHub | 内容 |
| --- | --- | --- |
| [001-rx-crate-plan.md](001-rx-crate-plan.md) | [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2) | トラッキング RFC。Inferno 準拠・Windows 第一級の受信クレート |
| [002-protocol-codecs.md](002-protocol-codecs.md) | [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3) | 制御・メディアの ser/de（パケット層） |
| [003-overlay-clock.md](003-overlay-clock.md) | [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) | プロセス内 overlay 時計（PTPv1 listen-only + メディア時刻） |
| [004-platform-sockets.md](004-platform-sockets.md) | [#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) | IF 固定ソケット、multicast、mDNS 5353、PTP 319/320 |
| [005-control-plane.md](005-control-plane.md) | [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) | mDNS + ARC/CMC。Dante Controller にデバイスが出る |
| [006-media-rx.md](006-media-rx.md) | [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7) | unicast subscribe、リングバッファ、`AudioBlock` |
| [007-tx.md](007-tx.md) | [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8) | TX チャンネル（最小 4 ms） |
| [008-cpal.md](008-cpal.md) | [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) | 既存 OS デバイスへの再生 / キャプチャ |
| [009-audio-host.md](009-audio-host.md) | [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) | ユーザー空間 ASIO / VST3 |

Inferno の参照ブランチは [001-rx-crate-plan.md](001-rx-crate-plan.md) の「Inferno の参照ブランチ」。
