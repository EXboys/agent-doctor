# Agent Doctor 中文说明

**Agent Doctor** 在本机诊断、修复并隔离多种 AI Agent runtime（OpenClaw、Hermes、DeepSeek Harness、Claude Code、Codex 等）。

默认是**个人本机运维**工具；需要团队合规时，再接 Evotown。边界见 [product-boundary.md](product-boundary.md)。

![Agent Doctor 桌面：环境健康度、Runtime 清单与待处理建议](../screenshot-desktop.png)

## 解决什么问题

同一个人可能同时安装：

- **OpenClaw** — 常驻助手、Skill、派活
- **Hermes** — 另一套 Agent 运行时
- **DeepSeek Harness** (`dsh`) — DeepSeek 官方 harness
- **Claude Code** — IDE/终端里的 coding agent
- **Codex CLI** — OpenAI coding agent

各自安装路径、配置、网关、Skill/MCP 和失败模式都不同。Agent 坏了时，需要快速判断是安装损坏、配置漂移、环境变量冲突，还是（团队场景下）网关/policy 不合规。

Agent Doctor 提供：

1. **发现** — 装了哪些、版本、配置在哪
2. **诊断** — `doctor` 检查安装、配置、网关与密钥来源
3. **修复** — 备份后按 playbook 修好（运维语义，不是切换管家）
4. **复验** — `ask` / 桌面 Ask，用该 runtime 再跑一遍确认
5. **隔离** — `workspace` 避免项目记忆 / MCP / Skill 串味
6. **（团队）合规** — 对照公司 baseline、Skill sync、policy、派活与审计

## 个人（C）与团队（B）

| | 个人（C） | 团队（B） |
|--|----------|----------|
| 入口 | CLI + 桌面「本机修好」 | 同客户端 + Evotown / 公司 profile |
| 价值 | 坏了能查、能修、项目不串味 | 合规、基线、同步、派活、审计 |
| LLM | 只走个人 Provider | 只走 Evotown 中转（互斥） |
| 不做 | 不做中转市场/用量看板 | 不依赖个人中转生态 |
| 增量 | 零配置就位、个人 provider、一键修好 | `setup` / `sync` / `policy` / `connect` + 审计导出 |

共同内核：**doctor / repair / workspace**。桌面「接线」页可在个人版 / 团队版之间切换。

## 桌面

托盘 + 窗口，和 CLI 共用同一套 Rust 内核。常见路径：**扫描 → 诊断 → 确认修复 → Ask 复验**。

| 页签 | 作用 |
|------|------|
| **Agents** | 环境健康度、Runtime 清单、诊断/修复抽屉 |
| **资源** | Skills / MCP 清单，Browser MCP 写入 Codex / Claude / Hermes / OpenClaw |
| **接线** | 个人 Provider 与 Evotown 团队模式（互斥） |
| **工作区** | 项目隔离切换、远程 VPS 只读 doctor、Hermes 场景预设 |

```bash
cd desktop && npm install && npm run tauri dev
```

## 和 ClawPanel 的区别

- [ClawPanel](https://github.com/qingchencloud/clawpanel) 侧重 OpenClaw + Hermes 图形化管理。
- Agent Doctor 侧重跨 runtime 的本机诊断、备份、修复与项目隔离；团队场景再叠加合规与 Evotown。

## 团队控制面（可选）

若团队部署了 Evotown，可通过 `setup` / `sync` / `policy pull` / `connect` 对接。见 [enterprise.md](../enterprise.md)。

## 当前状态

**v0.1.29** — CLI + 桌面本机运维已可用，尚未到 1.0。

已交付：五类 runtime 发现与探测、`repair --apply` / 回滚、桌面 Diagnose → Repair → Ask、工作区隔离、Browser MCP、个人/团队模式切换、Evotown 上线、SSH 只读 `remote` doctor。

尚未交付：合规报告导出、钥匙串存密钥、远程 SSH 修复、自动填入密钥。详见 [ROADMAP.md](../ROADMAP.md)。

## 常用命令

```bash
agent-doctor doctor
agent-doctor repair openclaw --apply
agent-doctor ask hermes "网关为什么连不上？"
agent-doctor workspace status
agent-doctor mcp configure codex
agent-doctor setup --url https://evotown.example.com --key evk_...
agent-doctor sync
agent-doctor policy pull
agent-doctor connect
```
