# CLI (`agent-doctor`)

Rust binary in the Agent Doctor workspace. Shared logic lives in `../crates/agent-doctor-core/`.

Agent Doctor is CLI-first so it can be used from onboarding scripts, IT support playbooks, and future repair automation.

## Build & run

```bash
# from repo root
cargo build -p agent-doctor
cargo run -p agent-doctor -- doctor
cargo run -p agent-doctor -- doctor --json
```

## Commands

| Command | Status |
|---------|--------|
| `doctor` | Implemented (OpenClaw, Hermes, DeepSeek Harness, Claude Code, Codex); `--explain` for AI diagnosis |
| `install <runtime>` | All registered runtimes: rule install when available; else / on failure → AI install |
| `repair <runtime>` | Probe + preview; `--apply` playbook + backup; `--rollback` |
| `ask <runtime>` | One-shot non-interactive prompt (Claude / Codex / DeepSeek Harness / Hermes / OpenClaw) |
| `mcp browser/status/configure` | Browser MCP via CDP; wire into Codex / Claude / Hermes / OpenClaw |
| `mode show/personal/team` | Exclusive personal provider vs Evotown team overlay |
| `profile list/init/use` | Implemented (Hermes model switching) |
| `config show` | Implemented (Hermes and other registered runtimes) |
| `workspace init/list/show/use/status/doctor/fix/matrix/direnv` | Per-project isolation for Hermes, Claude Code, Codex, OpenClaw |
| `remote host/project/doctor` | Agentless SSH read-only doctor |
| `setup --url --key` | Evotown/company profile → profile.env + evotown.agent.env + runtime configs |
| `sync` | Pull SkillHub bundle from Evotown (replaces `evotown-agent-setup.py sync`) |
| `policy pull` | Cache Evotown policies to `~/.config/evotown/policies-cache.json` |
| `register` | Register laptop engine; write `evi_` (replaces `evotown-agent-setup.py register`) |
| `connect` | WebSocket presence + inventory + job.assign execution |
| `preferred-runtime show/use` | Default local runtime for Evotown dispatch (`EVOTOWN_RUNTIME`) |
| `open <runtime>` | Open interactive CLI (Claude Code deep link, else system terminal) |

## Adapters

Runtime-specific code is in `crates/agent-doctor-core/src/adapters/`. See [../adapters/README.md](../adapters/README.md) for the adapter contract.

Repair safety primitives live in `agent-doctor-core::repair`: diagnostic facts are classified by sensitivity, redacted before AI analysis, and converted into typed repair actions before execution. Runtime probes live in `agent-doctor-core::probe` and currently check binaries, versions, PATH conflicts, config parse/schema issues, env conflicts, gateway reachability, and obvious MCP/Skills path references.
