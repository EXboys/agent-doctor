# Remote doctor (Agentless)

Diagnose **project + Agent runtime health on a remote VPS** over SSH.  
This is **not** a remote Provider switcher (unlike SSH-oriented config sync tools).

## Boundaries

| In scope | Out of scope |
|----------|--------------|
| Read-only `remote doctor` | Remote writes / `repair --apply` |
| One-time password → auto ed25519 key bootstrap | Persisting the password (never written) |
| OpenSSH BatchMode key auth thereafter | `sshpass` / interactive password doctor |
| Binary, config parse, gateway field, project traces | Provider preset push/pull |
| Local registry of hosts/projects | Installing `agent-doctor` on the VPS |

**Agentless** means the VPS does **not** need Agent Doctor installed. The local CLI drives checks via `ssh` (`run` / `cat` / `test`).

## Prerequisites

1. OpenSSH client on your machine (`ssh` / `ssh-keygen` on `PATH`).
2. For the **main path**: the VPS must allow **password authentication once** so Agent Doctor can install a public key. After bootstrap, only key auth is used (`BatchMode`; no password prompts).
3. For the **advanced path**: an existing `Host` alias in `~/.ssh/config` that already connects without a password (`ssh-agent` / keys / `ProxyJump`).

## Quick start (main path — bootstrap)

```bash
# Prefer env so the password does not land in shell history
export AD_SSH_PASSWORD='…'
agent-doctor remote host bootstrap prod-vps \
  --host 1.2.3.4 --user ubuntu --port 22 \
  --password-env AD_SSH_PASSWORD
unset AD_SSH_PASSWORD

# Register a project path on that host
agent-doctor remote project add prod-vps api --path /srv/api
# optional: --runtime hermes --runtime openclaw

# Read-only remote doctor (uses local ed25519 key)
agent-doctor remote doctor prod-vps/api
agent-doctor remote doctor prod-vps/api --json
agent-doctor remote doctor prod-vps/api --runtime hermes
```

What bootstrap does:

1. Generates `ed25519` under `…/agent-doctor/remote/keys/<id>` (Unix mode `0600`, empty passphrase).
2. Connects **once** with the password (OpenSSH + temporary `SSH_ASKPASS`; password never written to disk registry/secrets).
3. Appends the public key to remote `~/.ssh/authorized_keys`.
4. Verifies `ssh -i … -o BatchMode=yes true`.
5. Writes `hosts.yaml` with `hostname` / `user` / `port` / `identity_file`.

## Advanced path — existing SSH Host alias

```bash
agent-doctor remote host add prod-vps --ssh-config-host prod-vps
# Verify beforehand: ssh prod-vps true
```

`host list` labels entries as `managed` (bootstrap) or `legacy` (config Host).

## Registry layout

Via `dirs::config_dir()`:

- `…/agent-doctor/remote/hosts.yaml`
- Managed keys: `…/agent-doctor/remote/keys/<host_id>` (+ `.pub`)
- Reports: `…/agent-doctor/remote/reports/<host>/<project>/<ts>.json`

Password is **never** stored in yaml or the secrets store. Removing a host also deletes its managed key pair when present.

## What is checked

Per target runtime (default: openclaw, hermes, claude-code, codex):

1. SSH connectivity + remote `$HOME`
2. Project path is a directory + `pwd`
3. Binary on remote `PATH` + `--version`
4. Known config files under remote home — fetch + parse locally; record gateway/base_url (masked)
5. Light project traces (e.g. `.claude/`, `.mcp.json`, `.codex`)

## Desktop

「工作区」Tab 内有 **远程 VPS** 区块：主表单为「开通远程主机」（地址 / 用户 / 端口 / 一次性密码）；折叠项「已有 SSH Host 别名」保留高级入口。与 CLI 共用同一 `hosts.yaml`。密码仅本次 IPC，不落盘。

## Next (not in this release)

- `remote repair` preview / apply + remote backup
- Migrate local `probe` onto `ExecBackend` (drop parallel checks)
- `remote workspace` isolation
- Optional remote helper binary for heavier ops
- OS Keychain for managed private keys (today: local file + `0600`)
