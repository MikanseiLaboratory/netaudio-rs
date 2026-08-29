---
title: クロスプラットフォーム受信クレート — Inferno 準拠・Windows 第一級
labels: rfc, tracking
github: 2
---

# クロスプラットフォーム受信クレート — Inferno 準拠・Windows 第一級

[Inferno](https://github.com/teodly/inferno)（`inferno_aoip`）のデバイスモデルとパケットレイアウトを正として、**Windows / macOS / Linux で同じ API になる MIT クレート** を新規実装する。プロトコル知識と相互運用テストは Inferno / Dante Controller / パケット観測から取り、実装はオリジナル。

関連: [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3) プロトコル / [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) 時計 / [#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) ソケット / [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) 制御面 / [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7) メディア RX / [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8) TX / [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) cpal / [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) オーディオホスト

---

## 1. 何を作るか

Dante ネットワーク上で **受信デバイスとして見え、届いた PCM をアプリが汎用に扱える** Rust クレート。

| 項目 | v1 |
| --- | --- |
| 形態 | ライブラリ (`netaudio`)。制御・メディア・時計はプロセス内 |
| OS | **Windows を第一級**。macOS / Linux は同じ API |
| 音声 I/O | クレート境界は PCM。コールバック / Stream / リングバッファ |
| 遅延 | 設定可能。下限 **4 ms**（DVS と同じオーダー） |
| クロック | プロセス内 overlay（`Instant` / QPC）。PTPv1 listen-only とメディアパケット時刻 |

アプリ側の想定（クレートが知らなくてよい）:

- ファイルに書く
- 別プロトコルへ橋渡しする
- あとから `cpal` で実デバイスへ出す
- 自前の DSP / メータ

「汎用」= クレート境界は PCM とメディア時刻。OS 音声 API はアプリ、または [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) の責任。Inferno の `alsa_pcm_inferno` はプロトコルと ALSA が直結している。本クレートはそこで層を切る。

---

## 2. Inferno の参照ブランチ

参照先は Inferno upstream。

| Inferno ブランチ | 中身 | 本クレートでの使い方 |
| --- | --- | --- |
| **`dev`**（現行） | `protocol/`（ARC/CMC/flows/mcast）、`device_server/`（RX+TX+multicast+peaks）、`media_clock.rs` | **プロトコルとデバイス構成の正** |
| **`transmit`** | `flows_tx.rs`、`media_clock.rs`、TX スレッド | TX（[#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8)）のパケットとフロー制御 |
| **`tx_multicast`** | TX multicast | [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8) の後続 |
| **`master`** | capture、`cirb` | ポータブル受信の祖先。Windows 向けに薄い |
| **`stable`** | ALSA + `usrvclock-rs`。作者コメント: PTP 導入前の Inferno2pipe は Windows で動く見込みがあった | overlay 時計と PCM 境界を先に置けば Windows に乗る、という根拠 |
| `alsa-plugin` / `jack-compat` | Linux ユーザー空間ホスト | [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) の参考 |
| `binary_packets_refactor` | パケット整理 | [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3) の差分確認 |
| `latency_reporting` | DC への遅延・ピーク | [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6) の後続 |

作者コメント（[Inferno #3 Windows](https://github.com/teodly/inferno/issues/3)）:

- クロスプラットフォーム音声 I/O のあと残る Unix 依存は **時計** と RT スレッド優先度
- 別の Dante 時計の取り方があれば Statime は任意。精度は TX/RX latency で吸収
- 候補: **listen-only PTPv1**（ソフトタイムスタンプ）、またはメディアパケットの timestamp
- Windows の 1024 未満ポートは管理者不要が基本
- DAW 直結は OS ごとに書き直し。VST3 ならクロスプラットフォーム（リサンプル前提）

本クレート: **[#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4) の overlay 時計** と **[#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5) の std/socket2 ソケット** を先に固定し、音声境界は PCM API（[#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7)）。

---

## 3. デバイスの 3 面

Dante 互換デバイスは次の 3 面が揃って Controller からパッチできる。

```
[制御面]  mDNS広告 + ARC + CMC + info multicast
[メディア面] UDP flow（unicast/multicast）+ keepalive
[時刻面]  PTP ドメイン上のメディアクロック
```

受信専用でも制御面が要る。v1 の主経路は Controller パッチによる unicast subscribe。マルチキャスト購読は既存 mcast への join。

### 3.1 ポートとサービス（既知の既定値）

| 面 | 内容 | 既定 |
| --- | --- | --- |
| mDNS | `_netaudio-arc._udp.local` / `_netaudio-cmc._udp.local` | UDP 5353, 224.0.0.251 |
| ARC | ルーティング・チャンネル名・フロー数 | UDP 4440 |
| CMC | デバイス制御 | UDP 8800 |
| flows control | TX デバイスへ subscribe 要求 | UDP 4455 |
| info / heartbeat | 状態・ピーク・クロック情報 | 224.0.0.231:8702 / 224.0.0.233:8708, 自ポート 8700 |
| media | オーディオ flow | OS が割り当て（keepalive `0x13 0x37`、250 ms） |
| PTP | メディアクロック | UDP 319 / 320 |

同一 IP で複数インスタンスを動かすなら、Inferno の `ALT_PORT` と同様に制御ポートをずらす。v1 は単一インスタンス。

### 3.2 メディアパケット

UDP ペイロード先頭 9 バイトがヘッダ。残りがインターリーブ PCM。

- `[1..5)` 秒（BE u32）
- `[5..9)` その秒の中のサンプル位置（BE u32）
- `[9..]` サンプル。16/24/32-bit integer、チャンネルはインターリーブ
- 内部表現は **i32 に拡張**（Inferno と同じ。アプリへ出す境界も i32）

タイムスタンプは PTP メディアクロック上のサンプル位置。プロセス内受信の鍵。

### 3.3 時計と I/O 境界

| 層 | 本クレート | Inferno での対応（参照） |
| --- | --- | --- |
| overlay 時計 | `Instant` / QPC（[#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4)） | `usrvclock`、`media_clock.rs` |
| PTP / メディア時刻 | プロセス内 PTPv1 listen-only + メディアパケット時刻 | Statime |
| アプリ境界 | PCM API（[#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7)）。OS デバイスは [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) | `alsa_pcm_inferno` |
| 起動 | `Device::start` = ソケット + overlay | 時計デーモンと ALSA に直結 |

プロトコル本体（UDP、mDNS、ARC/CMC、リングバッファ）は標準ソケット。Inferno の `searchfire` には `cfg(windows)` がある。移植の本体は **時計と I/O 境界**。

---

## 4. アーキテクチャ

4 層。下から依存。`device` までの層は PCM とメディア時刻を扱う。

```
アプリ / cpal backend / オーディオホスト
        ↓  PCM + メディア時刻
[device]   ライフサイクル、設定、RX 購読 API
[media]    flow ソケット、keepalive、タイムスタンプ付きリングバッファ
[clock]    overlay clock（プロセス内 PTP slave またはメディア時刻）
[protocol] パケットの ser/de
```

詳細は [#3](https://github.com/MikanseiLaboratory/netaudio-rs/issues/3)〜[#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7)。

`rx_latency` / `tx_latency` はナノ秒。デフォルト 10 ms、**最小 4_000_000 ns**。下限未満は API で拒否する。

PTP ポート:

- **Windows** — 1024 未満の bind は管理者不要が基本
- **Linux / macOS** — 319/320 は privileged。bind 失敗を明示し、`CAP_NET_BIND_SERVICE` / root をドキュメントする。PTP はプロセス内

v1 の PTP は **PTPv1 slave（listen-only）**。Dante ハードウェアと噛み合わせる。PTPv2 / AES67 は Later。

v1 の TX チャンネル数は 0。設定型に `tx_latency` を先に持たせ、下限 4 ms を同じコードで検証する。

---

## 5. クロスプラットフォームで先に潰す点

1. **IF 固定** — ソケットはすべて `bind(local_v4)`。multicast は `IP_MULTICAST_IF` + join with interface IP（[#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5)）
2. **mDNS 5353** — macOS の mDNSResponder、Windows の Bonjour と共存。`SO_REUSEADDR` / `SO_REUSEPORT`。送れても受け取れないケースを最初に測る（[#5](https://github.com/MikanseiLaboratory/netaudio-rs/issues/5)）
3. **ファイアウォール** — 制御 UDP と ephemeral media の inbound。クレートは bind したポート一覧を返す
4. **IPv4** — Dante は IPv4。v1 は IPv4
5. **バイトオーダ** — メディア PCM はビッグエンディアン整数。ホストの i32 にしてからアプリへ

---

## 6. Later

| 段階 | 成果物 | 接続先 |
| --- | --- | --- |
| A. 今 | 汎用 PCM API | アプリのコールバック / ファイル / 他プロトコル |
| B. 次 | **cpal backend**（feature `cpal`）[#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9) | 既存の WASAPI / CoreAudio / ALSA /（環境により）ASIO **ホスト** |
| C. その次 | **ASIO ホスト**を明示（`asio-sys` 等の別 feature の可能性） | 既存 ASIO デバイス |
| D. 遠い | ユーザー空間 ASIO DLL または VST3 [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10) | DAW から見えるホスト |

`cpal` を足すときも `netaudio` 本体は PCM 境界のまま。`netaudio-cpal` または feature で `AudioBlock` をデバイスコールバックに接続する。クロックは:

- 再生: Dante メディア時計がマスター → 必要なら **リサンプリング** してデバイスコールバックへ
- 将来 TX: デバイス時計がマスターなら同様にリサンプルしてネットワークへ。4 ms はここのジッタ余裕

TX: `tx_latency` 設定可能、最小 4 ms。DVS 相当。

その他 Later: Dante Domain Manager、AES67 / ST 2110-30、PTP leader、ALT_PORT 複数インスタンス。

---

## 7. 技術スタック（v1）

| 用途 | 採用 | 理由 |
| --- | --- | --- |
| async 制御面 | tokio | タイムアウト・キャンセル |
| UDP multicast | tokio + socket2 | IF 指定、reuse、TTL |
| メディアスレッド | std::thread + nonblocking socket（mio は任意） | 制御面と分離 |
| 時刻 | `std::time::Instant` overlay | OS 非依存（Windows は QPC） |
| ser/de | 自前 or `byteorder` | プロトコル層を薄く保つ |
| log | `log` crate | アプリが backend を選ぶ |

依存は薄く。mDNS は自前か、**IF 指定できる** Windows 実装付き crate。ソケットは `std` / `socket2`。これが Windows 第一級の条件。

---

## 8. 実装フェーズ

**Phase 0 — 契約（この Issue）**  
API・範囲・時計方針を固定。

**Phase 1 — 制御面で DC に出る** → [#6](https://github.com/MikanseiLaboratory/netaudio-rs/issues/6)  
mDNS 広告 + ARC/CMC の最小セット。チャンネル数・名前・IP が見える。時計は後回しでも広告は出す。  
受け入れ: Dante Controller のデバイス一覧に名前が出る。

**Phase 2 — メディア RX + 汎用 API** → [#7](https://github.com/MikanseiLaboratory/netaudio-rs/issues/7)  
flows-control subscribe、unicast flow 受信、9 バイトヘッダ、リングバッファ、`AudioBlock`。時計は **メディアパケット駆動**。  
受け入れ: DC からパッチするとコールバックに PCM が届く。未パッチではブロックしない（無音の生成は任意）。

**Phase 3 — プロセス内 PTPv1 listen-only** → [#4](https://github.com/MikanseiLaboratory/netaudio-rs/issues/4)  
overlay を PTP にロック。DC のクロック表示が安定する。`rx_latency` 4 ms で連続受信が続く（負荷は別途測る）。  
受け入れ: ハードウェア 1 台以上 + DC で 1 時間オーダーの連続 RX。

**Phase 4 — TX（最小 4 ms）** → [#8](https://github.com/MikanseiLaboratory/netaudio-rs/issues/8)  
TX チャンネル、unicast 送信、`tx_latency`。アプリが PCM を書き込む API。

**Phase 5 — cpal feature** → [#9](https://github.com/MikanseiLaboratory/netaudio-rs/issues/9)  
既存デバイスへの再生。リサンプル方針をここで決める。

**Phase 6 — オーディオホスト** → [#10](https://github.com/MikanseiLaboratory/netaudio-rs/issues/10)  
ASIO DLL / VST3。この Issue の実装範囲の外。

---

## 9. 受け入れ条件（v1 = Phase 3 完了）

- Windows 10/11, macOS, Linux で同じ API がコンパイルできる
- `Device::start` だけで起動する
- Dante Controller から RX パッチできる
- アプリは `AudioBlock` だけで受信できる
- `rx_latency` を 4 ms 以上で設定できる
- ホストのシステム時刻は overlay の外（NTP はそのまま）
- 実装はオリジナル MIT。Inferno は upstream 参照と相互運用テスト

テスト相手: Dante Controller、実機、可能なら Inferno / DVS をピアにする。自動テストは loopback 2 インスタンス（同じホスト、ポート分離）。

---

## 10. 法務（実装前に読む）

- 本プロジェクトは Audinate 非公認。プロトコルは非公開で、実装はリバースエンジニアリングに基づく
- 特許が絡む。私的利用とバイナリ配布は別問題。README に disclaimer を置く
- Inferno は GPL-3.0-or-later OR AGPL-3.0-or-later。本リポジトリはオリジナルの MIT（fairlight-live-rs に合わせる）。パケットレイアウトの知識と相互運用テストは問題にしない
- 表示名は独自名（例: `Mikansei RX`）。互換であることは README で明示する

---

## 11. 調査ソース

- Inferno README / `inferno_aoip`（`dev` の `protocol/` と `device_server/`）
- Inferno Issue [#3 Windows](https://github.com/teodly/inferno/issues/3), [#7 clocking](https://github.com/teodly/inferno/issues/7)
- Inferno ブランチ `transmit`, `stable`, `master`, `tx_multicast`
- `usrvclock-rs`（overlay の式。実装は `Instant` / QPC）
- Statime（PTP ステートマシンの参照）
- [network-audio-controller](https://github.com/chris-ritsen/network-audio-controller)
