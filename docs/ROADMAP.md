# Roadmap

## P0 — CLI MVP

- [x] `agent-doctor doctor` — detect OpenClaw, Hermes, Claude Code, Codex; print config paths and gateway wiring
- [x] `agent-doctor doctor --explain` — AI diagnosis for runtimes with probe issues
- [x] `agent-doctor install <runtime>` — rule-based Hermes/OpenClaw install with install logs
- [x] `agent-doctor install <runtime> --explain` — install + AI failure/success explanation
- [x] Hermes model presets — `profile init/list/use`, `config show hermes`
- [x] Repair safety foundation — diagnostic sensitivity classes, redaction, typed actions, backup/audit report types
- [x] Runtime-specific read-only probes — binary, version, PATH conflicts, config parse/schema, env conflicts, gateway connectivity, MCP/Skills path references
- [x] Hermes deep probes — model schema, provider API key env, `.env` parse, duplicate/empty key checks, and `.env` file permissions
- [x] `agent-doctor repair <runtime>` — read-only probe + safe repair preview
- [x] `agent-doctor repair <runtime> --explain` — AI diagnosis from probe (no writes)
- [x] `agent-doctor repair <runtime> --apply` — backup → Hermes rule playbook → re-probe → audit (Hermes only for auto-fix today)
- [x] `agent-doctor repair <runtime> --rollback [--backup <id>]` — restore configs from `~/.config/agent-doctor/backups/`
- [x] `agent-doctor setup` — write `~/.config/agent-doctor/profile.env` + merge runtime configs + `~/.config/evotown/evotown.agent.env`
- [x] `agent-doctor sync` — Skill bundle sync from Evotown SkillHub (replaces `evotown-agent-setup.py sync`)
- [x] `agent-doctor policy pull` — cache policies from Evotown to `~/.config/evotown/policies-cache.json`
- [x] Desktop Evotown onboarding — URL/key form → setup + doctor + sync + policy pull
- [x] `agent-doctor connect` — WebSocket presence + inventory + **job.assign execution** (Claude/Codex CLI, OpenClaw/Hermes hooks)
- [ ] Company profile: `--url` + API key (generic non-Evotown control planes)
- [ ] Compliance report export for IT / DevEx support workflows
- [ ] `agent-doctor register` — migrate engine register from evotown-agent-setup.py

### Hermes repair (shipped vs planned)

| Capability | Status |
|------------|--------|
| Config backup before writes | Shipped |
| `.env` permissions → `600` | Shipped |
| Duplicate API key env entries deduped | Shipped |
| Fill empty `model.*` from active profile | Shipped |
| Missing API key → `.env` placeholder + local guide (no secret fill) | Shipped |
| Install Hermes via official `install.sh` / `install.ps1` when binary missing | Shipped |
| Install OpenClaw via official `openclaw.ai/install.sh` when binary missing | Shipped |
| Rollback from backup directory | Shipped (CLI + desktop) |
| AI-generated repair plans / free-form shell | Not planned for v1 |
| Auto-fill or upload API keys | Not planned |
| `install` / `update` runtime binaries | Hermes + OpenClaw: `install` command + repair playbook; Claude/Codex planned |
| OpenClaw / Claude / Codex rule playbooks | OpenClaw: install + config/schema/env/gateway fixes shipped; Claude/Codex planned |

## P1 — Desktop tray

- [x] Tauri menubar shell: tray menu, left-click to show window, run doctor
- [x] Runtime diagnosis panel with filterable checks and suggested fixes
- [x] Hermes: apply fixes, rollback, open API key guide
- [x] Evotown onboarding panel: connect URL/key, one-click doctor + sync + policy pull
- [x] Tray workspace switch + tooltip (`Agent Doctor · workspace: …`)
- [x] Tray compact status (small enhancement, not a product skin): tooltip shows health + active workspace + personal/team mode; refresh after doctor/repair/workspace switch; brief “busy” state while a tray action runs. No Dynamic Island chrome, theme pack, or floating HUD.
- [ ] Keychain storage for API keys (optional)

## P2 — Project workspaces

- [x] `agent-doctor workspace init` — register project; bind Hermes profile, Claude project dir, Codex CODEX_HOME, OpenClaw agent workspace
- [x] `agent-doctor workspace use` — activate workspace + write `active-workspace.env`
- [x] `agent-doctor workspace status` / `workspace doctor` — alignment and memory bleed checks
- [x] `agent-doctor workspace enter` / `env` / `match` — shell eval exports
- [x] `agent-doctor workspace hook install` — zsh/bash cd auto-align
- [x] `workspace use` backups before switching (Hermes/OpenClaw/project Claude settings)
- [x] MCP/skills snapshot per workspace (`workspaces/<name>/snapshots/`)
- [x] `workspace fix` — auto-rebind Hermes/OpenClaw, refresh env, restore project `.mcp.json`
- [x] Claude `~/.claude.json` user-scoped MCP detection in workspace doctor
- [x] Shell `cd` hook / direnv integration (fish + `workspace direnv`)
- [x] Desktop tray workspace switch + tooltip
- [x] `workspace use --restart-gateways` — Hermes/OpenClaw gateway restart

- [ ] Local policy evaluate before ingest
- [x] OpenClaw install via repair loop / playbook when `openclaw` binary missing
- [x] OpenClaw repair playbook: config schema, env permissions, gateway from profile, API key scaffold
- [x] OpenClaw MCP/skills path reference checks in workspace doctor
- [x] `workspace show` / `workspace matrix` — details and capability matrix
- [x] Desktop workspace doctor summary
- [x] Team baseline drift detection for gateway (company profile vs Hermes/OpenClaw)
- [x] C/B product boundary docs + durable `company-profile.env` (personal overlay does not pollute team baseline)
- [x] Desktop personal provider templates (URL/model fill-ins) + durable company baseline split
- [x] Exclusive personal/team mode switch (LLM path: personal provider XOR Evotown gateway)
- [x] OpenClaw default agent routing (`default: true`, `agents.defaults.workspace`)
- [x] Claude global MCP scaffold + migration hint on `workspace fix`
- [x] Codex CODEX_HOME isolation marker + doctor guard
- [ ] Team baseline drift — model provider / MCP / Skill full matrix
- [ ] SkillLite adapter (optional runtime)

## Optional integrations

- **[Evotown](https://github.com/EXboys/evotown)** — first-party control plane; see [enterprise.md](enterprise.md)
