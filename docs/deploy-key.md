# Cloud Agent 用 deploy key

公開鍵: [`deploy-key.pub`](deploy-key.pub)

```
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBIP8vvOVWV/MSTuYv5Le6AAfPbRH954txhUUBT8PP3l deploy-key:MikanseiLaboratory/netaudio-rs
```

Fingerprint: `SHA256:Ik7bM6q7cxZutS5rMF2C4JwUsUlBrCqo/7rgMlFj1vQ`

GitHub → Settings → Deploy keys → `netaudio-rs cloud-agent`（write）。秘密鍵は git に入っていない。Cursor secret `NETAUDIO_DEPLOY_KEY` または手元の `~/.ssh`。

```bash
install -m 700 -d ~/.ssh
printenv NETAUDIO_DEPLOY_KEY > ~/.ssh/netaudio-rs-deploy
chmod 600 ~/.ssh/netaudio-rs-deploy

GIT_SSH_COMMAND='ssh -i ~/.ssh/netaudio-rs-deploy -o IdentitiesOnly=yes' \
  git push origin HEAD
```
