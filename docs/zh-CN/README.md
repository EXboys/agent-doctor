# Agent Doctor 中文说明

**Agent Doctor** 在本机跟 Agent 对话，也能诊断、修复并隔离这些 runtime（OpenClaw、Hermes、DeepSeek Harness、Claude Code、Codex 等）。

默认安装包是**个人版**。团队合规是另一套安装包，接 Evotown。边界见 [product-boundary.md](product-boundary.md)。

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

1. **对话** — 在项目里问答，能用技能、工具、知识库，也能定时再发一句
2. **发现** — 装了哪些、版本、配置在哪
3. **诊断** — `doctor` 检查安装、配置、网关与密钥来源
4. **修复** — 备份后按 playbook 修好
5. **隔离** — `workspace` 让项目的记忆、工具、技能各管各的
6. **（团队）合规** — 对照公司 baseline、Skill sync、policy、派活与审计

## 个人版与团队版

两套安装包，应用内不切换。共同内核仍是 **doctor / repair / workspace**。

| | 个人版 | 团队版 |
|--|----------|----------|
| 安装包 | 默认，TeamUps | 企业定制 |
| 入口 | 问答窗口 + CLI | 同一套桌面能力 + Evotown |
| 价值 | 能对话、坏了能查能修、项目不串味 | 合规、基线、同步、派活、审计 |
| 模型 | 自己的模型服务 | 公司网关 |
| 增量 | 知识库、定时任务、飞书把话送到这台电脑 | `setup` / `sync` / `policy` / `connect` |

## 桌面

和 CLI 共用同一套 Rust 内核。两个窗口：

| 窗口 | 作用 |
|------|------|
| **问答** | 平时用的。左边是项目，中间是对话，旁边是当前文件夹。知识库、定时任务、技能和工具。登录后，飞书的一句话可以送到这台电脑。语音在设置里。 |
| **主页** | 托盘打开。健康度，然后是 Agent、资源、模型服务、项目。扫描、诊断、确认后修复。 |

```bash
cd desktop && npm install
npm run tauri:dev:personal   # 个人版
npm run tauri:dev:team       # 团队版
```

## 和 ClawPanel 的区别

- [ClawPanel](https://github.com/qingchencloud/clawpanel) 侧重 OpenClaw + Hermes 图形化管理。
- Agent Doctor 是本机上的对话，加上跨 runtime 的诊断、备份、修复和项目隔离。团队合规走单独的 Evotown 安装包。

## 团队版

团队安装包通过 `setup` / `sync` / `policy pull` / `connect` 对接 Evotown。见 [enterprise.md](../enterprise.md)。

## 当前状态

**尚未到 1.0** — 桌面问答和 CLI 已经能用。当前版本见 [GitHub Releases](https://github.com/EXboys/agent-doctor/releases)。

已交付：五类 runtime 发现与探测、`repair --apply` / 回滚、问答（项目、知识库、定时任务、技能和工具、语音、飞书发到这台电脑）、主页扫描和修复、个人版与团队版两套安装包、工作区隔离、Browser MCP、团队包上的 Evotown `setup` / `sync` / `connect`、SSH 只读 `remote` doctor。密钥写在本机 `settings.db` 旁边的 secrets 文件里（macOS / Linux 是 `secrets.json`，Windows 是 DPAPI），不走系统钥匙串。

桌面自动更新（国内 CDN 优先）：见 [desktop-auto-update.md](../desktop-auto-update.md)。

尚未交付：合规报告导出、远程 SSH 修复、自动填入密钥。详见 [ROADMAP.md](../ROADMAP.md)。

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
