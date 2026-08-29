# GitHub Issues への同期

計画の正本は [`docs/issues/`](issues/README.md)。この Cloud Agent の GitHub token は **Issues 書き込みが 403**（`Resource not accessible by integration`）。リポジトリの `has_issues` は true、open は 0。

PAT / GitHub App で `issues: write` が付いたら、以下で GitHub に載せる。front matter の `title` / `labels` は `gh` が無視するので明示する。

```bash
gh label create rfc --repo MikanseiLaboratory/netaudio-rs --description "Design / RFC" --color 0E8A16
gh label create tracking --repo MikanseiLaboratory/netaudio-rs --description "Tracking" --color 5319E7

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "クロスプラットフォーム受信クレート — Inferno 準拠・Windows 第一級" \
  --label rfc --label tracking \
  --body-file docs/issues/001-rx-crate-plan.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "protocol 層 — ARC / CMC / flows-control / メディアヘッダの ser/de" \
  --label enhancement \
  --body-file docs/issues/002-protocol-codecs.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "プロセス内 overlay 時計 — PTPv1 listen-only とメディアパケット時刻" \
  --label enhancement \
  --body-file docs/issues/003-overlay-clock.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "プラットフォームソケット — IF 固定、multicast、mDNS 5353、PTP ポート" \
  --label enhancement \
  --body-file docs/issues/004-platform-sockets.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "制御面 — mDNS + ARC/CMC で Dante Controller に出す" \
  --label enhancement \
  --body-file docs/issues/005-control-plane.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "メディア RX — subscribe、リングバッファ、AudioBlock API" \
  --label enhancement \
  --body-file docs/issues/006-media-rx.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "TX — unicast 送信と tx_latency（最小 4 ms）" \
  --label enhancement \
  --body-file docs/issues/007-tx.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "cpal feature — 既存 OS デバイスへの再生とキャプチャ" \
  --label enhancement \
  --body-file docs/issues/008-cpal.md

gh issue create --repo MikanseiLaboratory/netaudio-rs \
  --title "オーディオホスト — ユーザー空間 ASIO / VST3（Later）" \
  --label enhancement \
  --body-file docs/issues/009-audio-host.md
```

Markdown 内の `#002` などは GitHub 上で Issue 番号に読み替える。ファイル番号と Issue 番号を揃えること。

## Deploy key

公開鍵: [`docs/deploy-key.pub`](deploy-key.pub)

Fingerprint: `SHA256:Ik7bM6q7cxZutS5rMF2C4JwUsUlBrCqo/7rgMlFj1vQ`

GitHub → Settings → Deploy keys → `netaudio-rs cloud-agent`（write）。秘密鍵は git に入っていない。Cursor secret `NETAUDIO_DEPLOY_KEY` または手元の `~/.ssh`。
