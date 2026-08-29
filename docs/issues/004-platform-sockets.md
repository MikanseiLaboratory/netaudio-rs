---
title: プラットフォームソケット — IF 固定、multicast、mDNS 5353、PTP ポート
labels: enhancement
github: 5
---

# プラットフォームソケット — IF 固定、multicast、mDNS 5353、PTP ポート

Windows で Inferno 相当の制御・メディアを載せるためのソケット規約。**最初から IF 指定の IPv4 UDP** にする。

親: [#2](https://github.com/MikanseiLaboratory/netaudio-rs/issues/2)。[#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) / [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7) / [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) がこれを使う。

## 規約

1. **すべてのソケットを `bind(local_v4)`**。既定は `Bind::Ip`
2. **multicast join は特定 IF の IPv4** で行う（`IP_MULTICAST_IF` + `join_multicast_v4(group, if_ip)`）
3. **IPv4**（Dante の世界）
4. reuse: `SO_REUSEADDR`、可能な OS では `SO_REUSEPORT`
5. TTL / loop は Inferno の mDNS・info mcast に合わせる
6. クレートは **bind に成功したポート一覧** をアプリへ返す（ファイアウォール案内用）

`Bind::Unspecified` は明示したときだけ。

## mDNS 5353（制御面の最大リスク）

macOS は mDNSResponder、Windows は Bonjour が入っていると 5353 が埋まっている。

最初に測る:

- reuse 付きで bind → 広告を送れるか
- 同じソケットで他デバイスの mDNS を受け取れるか（送れても受け取れないケース）

次案: 5353 への send-only + 高ポート受信。システム DNS-SD API は IF 指定と TXT の再現性が弱いので後回し。Inferno `searchfire` の `cfg(windows)` を参考にする。

## PTP 319 / 320

[#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) と同じ。Windows は管理者なし bind が基本。失敗はエラーで返す。

## ファイアウォール（ドキュメント）

開ける UDP: 4455, 8700, 4400/4440, 8800, 5353、および OS が割り当てるメディア ephemeral。Inferno README の一覧に合わせ、実測で更新する。

## 受け入れ

- Windows 10/11 でマルチ NIC マシンの **指定 IF だけ** に広告と flow が乗る
- Linux / macOS でも同じ `Bind::Ip` API
- mDNS 5353 の send/recv 可否を README に実測結果で書く
