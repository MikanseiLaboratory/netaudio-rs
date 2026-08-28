# クロスプラットフォーム受信クレート — 技術要件と実装計画

Inferno ([teodly/inferno](https://github.com/teodly/inferno)) の実装調査を踏まえた、本リポジトリの方針。
**Inferno の fork ではない。** プロトコルの事実関係は Inferno / Dante Controller / パケット観測を参照し、実装は新規に書く。

---

## 1. 何を作るか

Dante ネットワーク上で **受信デバイスとして見え、届いた PCM をアプリが汎用に扱える** Rust クレート。

| 項目 | v1 の答え |
| --- | --- |
| 形態 | ライブラリ (`netaudio`)。デーモンプロセスは置かない |
| OS | Windows / macOS / Linux |
| 音声 I/O | **クレートの外**。コールバック / Stream / リングバッファで PCM を渡す |
| 仮想デバイス | **今はやらない**。後述の「計画」に残す |
| 遅延 | 設定可能。下限 **4 ms** でよい（DVS と同じオーダー） |
| クロック | プロセス内。`statime` / `ptp4l` / `usrvclock` に依存しない |

アプリ側の想定（クレートが知らなくてよい）:

- ファイルに書く
- 別プロトコルへ橋渡しする
- あとから `cpal` で実デバイスへ出す
- 自前の DSP / メータ

「汎用」の意味は **OS の音声 API に結び付けない** こと。Inferno の失敗点は、プロトコル実装が ALSA PCM プラグインと Unix クロックデーモン起動に直結していること。

---

## 2. 意図的にやらないこと（v1）

- 仮想サウンドカード（ALSA plugin / WDM / 仮想 ASIO DLL）
- 外部 PTP デーモン、Unix ドメインソケット、`/dev/ptp`
- OS システム時刻の操作（NTP 衝突は Inferno 既知の問題）
- Dante Domain Manager
- AES67 / ST 2110-30（メディアパケットは似ているが制御面が別）
- Inferno ソースのコピー（GPL/AGPL。本リポジトリは MIT）

---

## 3. Inferno から持ち込むべき事実（コピーしない）

### 3.1 役割分担

Dante 互換の「デバイス」は次の 3 面が揃って初めて Controller からパッチできる。

```
[制御面]  mDNS広告 + ARC + CMC + info multicast
[メディア面] UDP flow（unicast/multicast）+ keepalive
[時刻面]  PTP ドメイン上のメディアクロック
```

受信専用でも、**制御面がないとパッチできない**。
「RTP を待ち受けるだけ」では Dante ハードウェアは送ってこない。
（マルチキャスト購読は例外で、既存 mcast に join する道はある。v1 の主経路は Controller パッチによる unicast subscribe。）

### 3.2 ポートとサービス（既知の既定値）

| 面 | 内容 | 既定 |
| --- | --- | --- |
| mDNS | `_netaudio-arc._udp.local` / `_netaudio-cmc._udp.local` | UDP 5353, 224.0.0.251 |
| ARC | ルーティング・チャンネル名・フロー数 | UDP 4440 |
| CMC | デバイス制御 | UDP 8800 |
| flows control | TX デバイスへ subscribe 要求 | UDP 4455 |
| info / heartbeat | 状態・ピーク・クロック情報 | 224.0.0.231:8702 / 224.0.0.233:8708, 自ポート 8700 |
| media | オーディオ flow | OS が割り当て（keepalive `0x13 0x37`、250 ms） |
| PTP | メディアクロック | UDP 319 / 320 |

同一 IP で複数インスタンスを動かすなら、Inferno の `ALT_PORT` と同様に制御ポートをずらす。v1 では単一インスタンスでよい。

### 3.3 メディアパケット

UDP ペイロード先頭 9 バイトがヘッダ。残りがインターリーブ PCM。

- `[1..5)` 秒（BE u32）
- `[5..9)` その秒の中のサンプル位置（BE u32）
- `[9..]` サンプル。16/24/32-bit integer、チャンネルはインターリーブ
- 内部表現は **i32 に拡張**（Inferno と同じ。アプリへ出す境界も i32 でよい）

タイムスタンプは PTP メディアクロック上のサンプル位置。これが「デーモンなし受信」の鍵。

### 3.4 Inferno が Windows で落ちる理由（本クレートが避けるもの）

1. **usrvclock** — Unix datagram + `nix::ClockId` + `/dev/ptp` の `clock_adjtime`
2. **Statime デーモン必須** — キャプチャだけでも起動時に時計待ち（作者曰く設計上必須ではない）
3. **alsa_pcm_inferno** — `eventfd` / ALSA mmap。Windows に対応物がない
4. 起動パスが 1–3 に直結

プロトコル本体（UDP、mDNS、ARC/CMC、リングバッファ）は標準ソケットで書ける。mDNS 実装（searchfire）は既に `cfg(windows)` がある。つまり **移植の本体は時計と I/O 境界の切り方** であり、Dante デコードそのものではない。

---

## 4. アーキテクチャ（揃えるべきもの）

4 層。下から依存。上の層が OS 音声 API を知ってはいけない。

```
アプリ / 将来の cpal backend / 将来の仮想デバイス
        ↓  PCM + メディア時刻
[device]   ライフサイクル、設定、RX 購読 API
[media]    flow ソケット、keepalive、タイムスタンプ付きリングバッファ
[clock]    overlay clock（プロセス内 PTP slave またはメディア時刻）
[protocol] パケットの ser/de のみ。I/O なし
```

### 4.1 `protocol` — 純コーデック

- mDNS TXT / サービス型
- ARC / CMC / flows-control / info multicast
- メディア 9 バイトヘッダ

ここはユニットテストだけで閉じる。pcap を fixtures にする。

### 4.2 `clock` — デーモンを置き換える層（最重要）

**OS の時計は動かさない。** overlay だけ持つ。

```
overlay_ns = local_ns + shift + (local_ns - last_sync) * freq_scale
```

local は `std::time::Instant` / QPC。`CLOCK_TAI` も `/dev/ptp` も使わない。
これが Inferno 作者の「残る Linuxism はクロック」に対する答え。

v1 で実装する時計ソース（優先順）:

| 優先 | 方式 | 用途 | 精度 |
| --- | --- | --- | --- |
| 1 | プロセス内 PTPv1 listen-only（ソフトウェアタイムスタンプ） | Controller に時計ありと見せる、TX 将来、無音生成 | 4 ms バジェットで足りる |
| 2 | 受信メディアパケットのタイムスタンプで駆動 | PTP が取れないときの RX | フローが切れると時刻が止まる（Inferno2pipe と同じ） |

やらない:

- Statime / ptp4l への接続
- ハードウェアタイムスタンプ必須化
- システム時刻への step/freq steer

4 ms 下限の意味: DVS がソフトウェア時計で 4 ms を下限にしているのと同じ。ソフトウェア RX タイムスタンプの揺らぎを playout 遅延で吸収する。`rx_latency` / `tx_latency` はナノ秒で設定し、デフォルト 10 ms、**最小 4_000_000 ns**。これより低くは API で拒否してよい。

PTP ポート:

- **Windows** — 1024 未満の bind 制限はない。管理者不要が基本
- **Linux / macOS** — 319/320 は privileged。ライブラリは bind 失敗を明示し、`CAP_NET_BIND_SERVICE` / root をドキュメントする。別デーモンは立てない

PTPv2 は AES67 経路。v1 は **PTPv1 slave のみ** で Dante ハードウェアと噛み合わせる。v2 は後回し。

### 4.3 `media` — 受信の実体

- フローあたり UDP ソケット。DC がパッチすると flows-control で相手 TX に subscribe し、向こうから UDP が来る
- 250 ms keepalive（既知ペイロード `0x13 0x37`）
- パケット時刻でリングバッファに書く。アプリは「今のメディア時刻 − latency」から読む
- multicast flow は `join_multicast_v4` を **特定 IF の IPv4** で行う（Windows で Default IF にすると死ぬ）

受信スレッドと Tokio 制御面は分ける。Inferno が mio + 専用スレッドにしている理由（リアルタイム、キュー滞留）は正しい。v1 は `thread + nonblocking UDP` でよい。優先度は best-effort（Windows は `THREAD_PRIORITY_TIME_CRITICAL` 相当、失敗しても動作）。

### 4.4 `device` — アプリが触る面

望ましい形（実装前の契約）:

```rust
let device = Device::start(Settings {
    name: "Mikansei RX".into(),
    bind: Bind::Ip(local_v4),
    rx_channels: 8,
    sample_rate: 48_000,
    rx_latency: Duration::from_millis(4), // 下限 4ms
    ..Default::default()
}).await?;

// Dante Controller からこのデバイスの RX にパッチできる

device.set_rx_handler(|block: AudioBlock| {
    // block.media_time  : メディアクロック上の先頭サンプル
    // block.sample_rate
    // block.channels    : planar &[ &[Sample] ]  または interleaved
});
```

必須設定: `BIND_IP`（IF 名でも可）、`NAME`、`RX_CHANNELS`、`SAMPLE_RATE`、`RX_LATENCY`。
Device ID は未指定なら IP + process から安定生成（Inferno と同趣旨。状態キーになる）。

v1 の TX チャンネル数は 0 でよい。ただし設定型に `tx_latency` を先に持たせ、下限 4 ms を同じコードで検証する。

---

## 5. クロスプラットフォームで最初に潰す落とし穴

1. **IF 固定**  
   マルチ NIC が普通。ソケットはすべて `bind(local_v4)`。multicast は `IP_MULTICAST_IF` + join with interface IP。`0.0.0.0` 放置は Windows で壊れる。

2. **mDNS 5353**  
   macOS は mDNSResponder、Windows は Bonjour が入っていると 5353 が埋まっている。`SO_REUSEADDR` / `SO_REUSEPORT` と、送れても受け取れないケースを最初に検証する。だめならシステム DNS-SD ではなく、5353 への send-only + 高ポート受信の可否を測る。ここが制御面の最大リスク。

3. **ファイアウォール**  
   制御 UDP と ephemeral media ポートの inbound。ドキュメントに明示。アプリ側の責任でも、クレートは bind したポート一覧を返せるようにする。

4. **IPv4 のみ**  
   Dante は IPv4。IPv6 はやらない。

5. **バイトオーダ**  
   メディア PCM はビッグエンディアン整数。ホストエンディアンで i32 に直してからアプリへ。

---

## 6. 計画に残すもの（今は実装しない）

仮想デバイスは **「cpal を使えば ASIO 仮想デバイスになる」ではない。** 別物なので分けて書く。

| 段階 | 何 | 何でない |
| --- | --- | --- |
| A. 今 | 汎用 PCM API | 音声デバイス |
| B. 次 | **cpal backend**（feature `cpal`）: 受信 PCM を既存の出力デバイスへ出す / 既存入力を将来の TX へ | 仮想デバイスではない。WASAPI / CoreAudio / ALSA /（環境により）ASIO **ホスト** に乗る |
| C. その次 | **ASIO ホスト** を明示サポート。cpal の ASIO は弱いので `asio-sys` 等を別 feature にする可能性が高い | DAW から見えるドライバではない |
| D. 遠い | 仮想デバイス。ユーザー空間 ASIO DLL（署名不要）が現実的。WDM は署名が要り本リポジトリの対象外 | |

B を足すときも、`netaudio` 本体は cpal を知らない。`netaudio-cpal` または feature で `AudioBlock` をデバイスコールバックに接続するだけ。クロックは:

- 再生: Dante メディア時計がマスター → 必要なら **リサンプリング** してデバイスコールバックへ（デバイスがマスターのとき）
- 将来 TX: デバイス時計がマスターなら同様にリサンプルしてネットワークへ出す。4 ms はここのジッタ余裕

TX を足すときの制約: `tx_latency` 設定可能、最小 4 ms。DVS 相当。1 ms は狙わない。

---

## 7. 技術スタック（v1 で揃えるもの）

| 用途 | 採用 | 理由 |
| --- | --- | --- |
| async 制御面 | tokio | 既存資産・タイムアウト・キャンセル |
| UDP multicast | tokio + socket2 | IF 指定、reuse、TTL |
| メディアスレッド | std::thread + nonblocking socket（mio は任意） | 制御面と分離 |
| 時刻 | `std::time::Instant` overlay | OS 非依存 |
| ser/de | 自前 or `byteorder` / `binary-serde` | Inferno 依存を増やさない |
| log | `log` crate | アプリが backend を選ぶ |
| 禁止 | `nix`, `alsa`, `libc` の Unix API, Unix datagram, `/dev/ptp` | クロスプラットフォームを壊す |

依存は薄く。mdns は自前か、Windows 実装がある crate を **IF 指定できるものだけ** 使う。

---

## 8. 実装フェーズ

**Phase 0 — 契約（この Issue）**  
API・非目標・時計方針を固定。

**Phase 1 — 制御面で DC に出る**  
mDNS 広告 + ARC/CMC の最小セット。チャンネル数・名前・IP が見えればよい。時計は「なし」でも広告は出す。  
受け入れ: Dante Controller のデバイス一覧に名前が出る。

**Phase 2 — メディア RX + 汎用 API**  
flows-control subscribe、unicast flow 受信、9 バイトヘッダ、リングバッファ、`AudioBlock` コールバック。時計は **メディアパケット駆動** でよい。  
受け入れ: DC からパッチするとコールバックに PCM が届く。未パッチではブロックしない（無音を捏造しなくてよい）。

**Phase 3 — プロセス内 PTPv1 listen-only**  
overlay を PTP にロック。DC のクロック表示が破綻しないこと。`rx_latency` 4 ms で連続受信が落ちないこと（負荷は別途測る）。  
受け入れ: ハードウェア 1 台以上 + DC で 1 時間オーダーの連続 RX。

**Phase 4 — TX（最小 4 ms）**  
TX チャンネル、unicast 送信、`tx_latency`。仮想デバイスはまだ不要。アプリが PCM を書き込む API。

**Phase 5 — cpal feature**  
既存デバイスへの再生。リサンプル方針をここで決める。

**Phase 6 — ASIO ホスト / 仮想デバイス**  
調査 Issue を別途切る。この Issue の実装範囲外。

---

## 9. 受け入れ条件（v1 = Phase 3 完了）

- Windows 10/11, macOS, Linux で同じ API がコンパイルできる
- 外部プロセスなしで起動する
- Dante Controller から RX パッチできる
- アプリは `AudioBlock` 以外の音声 API を知らなくて受信できる
- `rx_latency` を 4 ms 以上で設定できる
- OS の時刻を変更しない
- Inferno のソースを含まない

テスト相手: Dante Controller、実機、可能なら Inferno / DVS をピアにする。自動テストは loopback 2 インスタンス（同じホスト、ポート分離）。

---

## 10. 法務（実装前に読む）

- Audinate 非公認。プロトコルは非公開で、実装はリバースエンジニアリングに基づく
- 特許が絡む。私的利用とバイナリ配布は別問題。README に disclaimer を置く
- Inferno は GPL-3.0-or-later OR AGPL-3.0-or-later。**コードを持ってこない。** パケットレイアウトの知識と相互運用テストは問題にしない
- 本リポジトリは MIT（fairlight-live-rs に合わせる）
- 偽装製品にしない。表示名は公式 Dante を名乗らない

---

## 11. 調査ソース

- Inferno README / `inferno_aoip`（制御・メディア・時計の接続）
- Inferno Issue [#3 Windows](https://github.com/teodly/inferno/issues/3), [#7 clocking](https://github.com/teodly/inferno/issues/7)
- `usrvclock-rs`（overlay の意味。Unix 実装は使わない）
- Statime（デーモンとしては使わない。PTP ステートマシンの参照にはなり得る）
- [network-audio-controller](https://github.com/chris-ritsen/network-audio-controller)
