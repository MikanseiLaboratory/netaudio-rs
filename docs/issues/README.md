# Issues（正本）

GitHub Issues API が Cloud Agent から書けないため、計画の正本はここ。番号とタイトルは GitHub に載せるときも同じにする。

表記は **何を作るか** で書く。除外リストは「Later」に置く。Audinate 非公認の法務文だけは独立した disclaimer として残す。

| # | ファイル | 内容 |
| --- | --- | --- |
| 001 | [001-rx-crate-plan.md](001-rx-crate-plan.md) | トラッキング RFC。Inferno 準拠・Windows 第一級の受信クレート |
| 002 | [002-protocol-codecs.md](002-protocol-codecs.md) | 制御・メディアの ser/de（I/O なし） |
| 003 | [003-overlay-clock.md](003-overlay-clock.md) | プロセス内 overlay 時計（PTPv1 listen-only + メディア時刻） |
| 004 | [004-platform-sockets.md](004-platform-sockets.md) | IF 固定ソケット、multicast、mDNS 5353、PTP 319/320 |
| 005 | [005-control-plane.md](005-control-plane.md) | mDNS + ARC/CMC。Dante Controller にデバイスが出る |
| 006 | [006-media-rx.md](006-media-rx.md) | unicast subscribe、リングバッファ、`AudioBlock` |
| 007 | [007-tx.md](007-tx.md) | TX チャンネル（最小 4 ms） |
| 008 | [008-cpal.md](008-cpal.md) | 既存 OS デバイスへの再生 / キャプチャ |
| 009 | [009-audio-host.md](009-audio-host.md) | ユーザー空間 ASIO / VST3（遠い） |

Inferno 側の参照ブランチは 001 の「Inferno の参照ブランチ」にまとめた。
