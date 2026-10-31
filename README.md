# Agent Doctor

**Talk to the agents on this computer, and repair them when they break.**

Agent Doctor discovers OpenClaw, Hermes, DeepSeek Harness, Claude Code, Codex, and related runtimes. The desktop chat works inside a project. When something is misconfigured, redacted probes find it, and repairs run from backups, typed actions, and an audit report.

![Agent Doctor desktop — environment health, runtime inventory, and suggested repairs](docs/screenshot-desktop.png)

**Personal (default package):** the desktop chat plus the CLI. Talk to the agent in a project, keep a knowledge base, and schedule a sentence for later. When an agent breaks, scan, diagnose, and repair it on this computer. Wire your own model service if needed. This is not a provider marketplace.

**Team (separate package):** [Evotown](https://github.com/EXboys/evotown) for a compliance baseline, skill sync, policy, dispatch, and audit. Personal and team ship as two installers. The app does not switch between them.

Product boundary: [docs/product-boundary.md](docs/product-boundary.md) · 中文：[docs/zh-CN/product-boundary.md](docs/zh-CN/product-boundary.md)

```bash
agent-doctor doctor                              # Diagnose: installed runtimes, config paths, gateway wiring
agent-doctor repair hermes                       # Repair: probes + safe preview (no writes)
agent-doctor repair hermes --apply               # Backup, rule playbook, re-probe, audit
agent-doctor repair hermes --rollback            # Restore latest backup (or --backup <id>)
agent-doctor ask hermes "why is the gateway down?"  # One-shot check via the runtime
agent-doctor workspace status                    # Project isolation / memory bleed
agent-doctor setup --url ... --key ...           # Onboard: apply company profile
agent-doctor connect                             # Stay online on Evotown (WS presence + inventory)
```

[License: MIT](LICENSE) · [Roadmap](docs/ROADMAP.md) · [Repair safety](docs/repair-safety.md)

---

## Desktop

The Tauri app uses the same Rust core as the CLI. Two windows:

| Window | What it does |
| --- | --- |
| **Chat** | The daily window. Projects on the left, the conversation in the middle, the current folder on the side. Knowledge base, scheduled tasks, skills and tools. A signed-in computer can take a sentence from Feishu. Voice is in settings. |
| **Home** | Tray window. Health, then Agent, resources, model service, and projects. Scan, diagnose, confirm a repair. |

Personal builds show your model service. Team builds show the company gateway. Each package has its own update channel.

```bash
cd desktop && npm install
npm run tauri:dev:personal   # TeamUps
npm run tauri:dev:team       # enterprise
```

See [desktop/README.md](desktop/README.md).

---

## Team package

The team installer uses **Evotown** as its control plane, on top of doctor / repair / workspace:

```bash
agent-doctor setup --url https://your-evotown.example.com --key evk_...
agent-doctor sync          # Pull private SkillHub bundle
agent-doctor policy pull   # Cache policies locally
agent-doctor register --bootstrap-token <IT-token>  # Write evi_ for connect
agent-doctor connect       # WebSocket online + inventory + job.assign
```

See [docs/enterprise.md](docs/enterprise.md) and [docs/product-boundary.md](docs/product-boundary.md).

---

## Status

**Pre-1.0** — desktop chat and the CLI are usable. Current release: [GitHub Releases](https://github.com/EXboys/agent-doctor/releases).

**Shipped:** discovery and probes for OpenClaw, Hermes, DeepSeek Harness, Claude Code, and Codex; `repair --apply` / `--rollback` with backups; desktop chat (projects, knowledge, scheduled tasks, skills and tools, voice, Feishu to this computer); home window for scan and repair; separate personal and team installers; workspace isolation; Browser MCP; Evotown `setup` / `sync` / `connect` on the team package; read-only `remote` doctor over SSH.

**Not yet:** compliance report export, remote repair over SSH, or auto-filling secrets. API keys are stored in a **local secrets file** next to `settings.db` (`secrets.json` on macOS/Linux with mode `0600`; DPAPI-protected `secrets.dpapi` on Windows — no Keychain / Credential Manager prompts). Legacy `.env` migrates once. See [docs/ROADMAP.md](docs/ROADMAP.md).

Repair writes stay typed: backup first, no free-form shell, secrets redacted before AI analysis. Missing keys get a `.env` placeholder and a local guide — you paste the secret. — [docs/repair-safety.md](docs/repair-safety.md).

---

## Why Agent Doctor?

Developers and teams increasingly run **several** local AI agent runtimes:

| Runtime | Typical config |
| --- | --- |
| [OpenClaw](https://github.com/openclaw/openclaw) | `~/.openclaw/openclaw.json` |
| [Hermes Agent](https://github.com/nousresearch/hermes-agent) | `~/.hermes/config.yaml` |
| [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (`dsh`) | `~/.dsh/settings.yaml` |
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/settings.json` |
| Codex CLI | `~/.codex/config.toml` |

Each runtime has its own install path, gateway settings, skills manifest, policy surface, and failure modes. Agent Doctor gives you **one** local client to answer:

- What is installed on this laptop?
- Where do configs live?
- Why did this agent stop working, and can we repair it safely?
- Are projects isolated (workspace), or is memory/MCP bleeding across them?
- *(Team)* Are runtimes pointed at the approved company gateway / baseline?
- *(Team)* What drifted from the team profile, and can we restore compliance?

```text
  Your laptop
 ┌──────────────────────────────────┐
 │ Agent Doctor                     │
 │ chat · doctor · repair · project │
 └──────────────┬───────────────────┘
                │
   OpenClaw · Hermes · DeepSeek Harness · Claude Code · Codex
```

---

## Relationship to other tools

| Project | Scope |
| --- | --- |
| **[ClawPanel](https://github.com/qingchencloud/clawpanel)** | Rich GUI for OpenClaw + Hermes |
| **[ClawPal](https://github.com/lay2dev/clawpal)** | OpenClaw desktop config companion |
| **Agent Doctor** | **Local chat with the agents on this computer, plus diagnosis, backup, repair, and project isolation. Team compliance is a separate Evotown package.** |

---

## 中文

**Agent Doctor** 在本机跟 Agent 对话，也能 **诊断、修复、隔离** 这些 Runtime（OpenClaw、Hermes、DeepSeek Harness、Claude Code、Codex 等）。

- **个人版**：单独安装包。问答、项目、知识库、定时任务；坏了能查、能修。模型服务自己填。
- **团队版**：另一套安装包，接 Evotown。合规基线、同步、策略、派活、审计。两套包各自更新。

桌面有两个窗口：问答是平时用的；托盘里的主页用来扫描和修复。产品边界：[docs/zh-CN/product-boundary.md](docs/zh-CN/product-boundary.md)。

```bash
agent-doctor doctor                              # 诊断
agent-doctor repair hermes                       # 修复预览（不写文件）
agent-doctor repair hermes --apply               # 备份 + 规则修复 + 复检
agent-doctor repair hermes --rollback            # 从备份恢复
agent-doctor ask hermes "网关为什么连不上？"
agent-doctor workspace status
agent-doctor setup --url ... --key ...           # 团队就位（Evotown）
```

团队可选：`sync`、`policy pull`、`connect` — 见 [docs/enterprise.md](docs/enterprise.md)。完整中文说明：[docs/zh-CN/README.md](docs/zh-CN/README.md)。

---

## Development

```bash
# CLI
cargo run -p agent-doctor -- doctor

# Local CI checks (fmt / clippy / test)
make check
# or: ./scripts/check.sh cli

# Desktop (requires Node.js)
cd desktop && npm install && npm run tauri:dev:personal
```

See [docs/development.md](docs/development.md), [docs/ROADMAP.md](docs/ROADMAP.md), [docs/install.md](docs/install.md), [cli/README.md](cli/README.md), [desktop/README.md](desktop/README.md), and [CONTRIBUTING.md](CONTRIBUTING.md).

## Install

Prebuilt CLI and desktop bundles are published to [GitHub Releases](https://github.com/EXboys/agent-doctor/releases). Mainland China auto-update prefers the CDN endpoint — see [docs/desktop-auto-update.md](docs/desktop-auto-update.md).

```bash
# Latest CLI (pick the pattern for your OS — see docs/install.md)
gh release download --repo EXboys/agent-doctor --pattern 'agent-doctor-*-macos-arm64.tar.gz'
tar -xzf agent-doctor-*-macos-arm64.tar.gz && chmod +x agent-doctor
./agent-doctor doctor
```

## License

MIT — see [LICENSE](LICENSE).
