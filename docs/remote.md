# Remote doctor (Agentless)

Diagnose **project + Agent runtime health on a remote VPS** over SSH.  
This is **not** a remote Provider switcher (unlike SSH-oriented config sync tools).

## Boundaries

| In scope (MVP) | Out of scope |
|----------------|--------------|
| Read-only `remote doctor` | Remote writes / `repair --apply` |
| OpenSSH key / agent auth (`BatchMode`) | Password / `sshpass` |
| Binary, config parse, gateway field, project traces | Provider preset push/pull |
| Local registry of hosts/projects | Installing `agent-doctor` on the VPS |

**Agentless** means the VPS does **not** need Agent Doctor installed. The local CLI drives checks via `ssh` (`run` / `cat` / `test`).

## Prerequisites

1. OpenSSH client on your machine (`ssh` on `PATH`).
2. A working `Host` alias in `~/.ssh/config` (or equivalent) that connects **without a password prompt** (`ssh-agent` / keys / `ProxyJump` as usual).
3. Verify: `ssh <Host> true`

## Quick start

```bash
# Register host (ssh_config_host = Host alias from ~/.ssh/config)
agent-doctor remote host add prod-vps --ssh-config-host prod-vps

# Register a project path on that host
agent-doctor remote project add prod-vps api --path /srv/api
# optional: --runtime hermes --runtime openclaw

# Read-only remote doctor
agent-doctor remote doctor prod-vps/api
agent-doctor remote doctor prod-vps/api --json
agent-doctor remote doctor prod-vps/api --runtime hermes
```

Registry file (via `dirs::config_dir()`):

- `…/agent-doctor/remote/hosts.yaml`
- Reports: `…/agent-doctor/remote/reports/<host>/<project>/<ts>.json`

## What is checked

Per target runtime (default: openclaw, hermes, claude-code, codex):

1. SSH connectivity + remote `$HOME`
2. Project path is a directory + `pwd`
3. Binary on remote `PATH` + `--version`
4. Known config files under remote home — fetch + parse locally; record gateway/base_url (masked)
5. Light project traces (e.g. `.claude/`, `.mcp.json`, `.codex`)

## Desktop

「工作区」Tab 内有 **远程 VPS** 区块：登记 Host / 项目、列表检查（只读 doctor）。与 CLI 共用同一 `hosts.yaml`。

## Next (not in MVP)

- `remote repair` preview / apply + remote backup
- Migrate local `probe` onto `ExecBackend` (drop parallel checks)
- `remote workspace` isolation
- Optional remote helper binary for heavier ops
