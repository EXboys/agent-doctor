# Workspace capability matrix

Agent Doctor **workspaces** add per-project isolation on top of native runtime behavior.  
Tiers: **L3** = native or near-native isolation; **L2** = overlay / config-level isolation.

Run `agent-doctor workspace matrix` for CLI output (`--json` for automation).  
Matrix version: **workspace-v1.6**.

## Runtime × dimension

| Runtime | Dimension | Native | Agent Doctor | Tier |
|---------|-----------|--------|--------------|------|
| Hermes | Memory / sessions | Per profile | Dedicated profile + `HERMES_HOME` | L3 |
| Hermes | Gateway | `gateway.lock` per profile | Doctor + `--restart-gateways` | L3 |
| Claude Code | Memory | Project hash memory | `.claude/` scaffold + cwd | L3 |
| Claude Code | MCP | user / project / claude.json | Snapshot + global bleed doctor | L3 |
| Claude Code | Global MCP migration | Manual | `fix` scaffolds `.mcp.json` + hint doc | L3 |
| Claude Code | Skills | `.claude/skills/` | Snapshot project + Hermes profile skills | L3 |
| Codex | Memory | Single `~/.codex` | Per-workspace `CODEX_HOME` | L2 |
| Codex | Isolation guard | None | Marker file; doctor **fail** if aliased to global | L2 |
| OpenClaw | Agent workspace | `agents.list[].workspace` | Bind workspace dir | L2 |
| OpenClaw | Default routing | `default: true` + bindings | Sets default agent + `agents.defaults.workspace` | L2 |
| OpenClaw | MCP/skills paths | Config refs | Doctor scans missing paths | L2 |
| Cross | Shell env | Manual | env file + hooks + direnv | — |
| Cross | Switch safety | None | Backup + `workspace fix` | — |
| Cross | Company baseline | None | Doctor vs `profile.env` gateway | — |
| Cross | Desktop | N/A | Tray + picker + doctor | — |

## Gap status (v1.6)

| Former gap | Status |
|------------|--------|
| OpenClaw gateway routing | **Closed** — default agent + `agents.defaults.workspace` on use/fix |
| Claude global MCP | **Mitigated** — detect + scaffold + migration hint; user removes global servers manually |
| Codex per-repo memory | **Mitigated (L2)** — isolated `CODEX_HOME` + marker + fail on global alias |
| Team baseline drift | **Partial** — gateway drift checks vs company profile |
| OpenCode / SkillLite | **Open** — not in workspace v1 |
| DeepSeek Harness, Qoder, WorkBuddy, Cursor | **Open** — descriptor has no workspace bind; `workspace matrix` lists them as unsupported |
| Hermes skills | **Closed** — profile skills snapshot |

## vs CC Switch / manual

| Capability | Manual / CC Switch | Agent Doctor |
|------------|-------------------|--------------|
| Hermes per-project profile | Manual | `workspace init` |
| Codex memory isolation | ❌ | ✅ `CODEX_HOME` + guard |
| OpenClaw default routing | Manual openclaw.json | ✅ auto on use/fix |
| MCP bleed | ❌ | ✅ doctor + fix scaffold |
| Company gateway drift | ❌ | ✅ baseline checks |
| Switch backup | ❌ | ✅ |

## Commands

| Goal | Command |
|------|---------|
| Register | `workspace init` |
| Activate | `workspace use <name>` |
| Check | `workspace doctor` |
| Fix | `workspace fix` |
| Matrix | `workspace matrix` |

See [workspace.md](workspace.md).
