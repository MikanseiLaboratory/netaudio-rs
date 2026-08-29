# 後続エージェント向け

この Cloud Agent には GitHub 資格情報がなく、リモート作成・Issue 投稿・push はしていない。
**GitHub Issue は作らない。** `docs/issues/` の Markdown を `gh issue create` する。

## 1. private リポジトリ

```bash
gh repo create MikanseiLaboratory/netaudio-rs \
  --private \
  --description "Unofficial Dante-compatible AoIP library (receive-first, cross-platform)" \
  --source=. \
  --remote=origin \
  --disable-wiki \
  --disable-issues=false
```

まだ `git push` しない。先に deploy key を登録する。

## 2. Deploy key（write）

公開鍵: [`docs/deploy-key.pub`](deploy-key.pub)

```
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBIP8vvOVWV/MSTuYv5Le6AAfPbRH954txhUUBT8PP3l deploy-key:MikanseiLaboratory/netaudio-rs
```

Fingerprint: `SHA256:Ik7bM6q7cxZutS5rMF2C4JwUsUlBrCqo/7rgMlFj1vQ`

GitHub → repo Settings → Deploy keys → Add deploy key:

- Title: `netaudio-rs cloud-agent`
- Allow write access: **on**
- 中身は `docs/deploy-key.pub`

秘密鍵は git に入っていない。起動元が Cursor secret `NETAUDIO_DEPLOY_KEY`（OpenSSH private key）として渡すか、手元の `~/.ssh` に置く。

```bash
# secret から鍵を出す場合
install -m 700 -d ~/.ssh
printenv NETAUDIO_DEPLOY_KEY > ~/.ssh/netaudio-rs-deploy
chmod 600 ~/.ssh/netaudio-rs-deploy
ssh-keygen -y -f ~/.ssh/netaudio-rs-deploy > ~/.ssh/netaudio-rs-deploy.pub

cat >> ~/.ssh/config << 'EOF'
Host github.com-netaudio
  HostName github.com
  User git
  IdentityFile ~/.ssh/netaudio-rs-deploy
  IdentitiesOnly yes
EOF

git remote add origin git@github.com-netaudio:MikanseiLaboratory/netaudio-rs.git
# すでに origin があるなら:
# git remote set-url origin git@github.com-netaudio:MikanseiLaboratory/netaudio-rs.git

GIT_SSH_COMMAND='ssh -i ~/.ssh/netaudio-rs-deploy -o IdentitiesOnly=yes' \
  git push -u origin main
```

`gh` が使えるなら PAT / GitHub App で `gh repo create` したあと、push だけこの鍵でもよい。

## 3. Issue を GitHub に載せる

```bash
gh issue create \
  --repo MikanseiLaboratory/netaudio-rs \
  --title "クロスプラットフォーム受信クレート — 技術要件と実装計画" \
  --label rfc --label tracking \
  --body-file docs/issues/001-rx-crate-plan.md
```

front matter の `title` / `labels` は `gh` が無視するので、上のとおり明示する。
ラベルが無ければ先に作る:

```bash
gh label create rfc --repo MikanseiLaboratory/netaudio-rs --description "Design / RFC" --color 0E8A16
gh label create tracking --repo MikanseiLaboratory/netaudio-rs --description "Tracking" --color 5319E7
```

このリポジトリでは GitHub Issue をソースにしない。追加の計画は `docs/issues/00N-*.md` に足す。
