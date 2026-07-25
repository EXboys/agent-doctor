# Evotown 集成（第一方）

Agent Doctor 是 **[Evotown](https://github.com/EXboys/evotown) 的官方本机客户端**：发现 runtime、诊断漂移、备份配置、修复到团队 baseline，并从 Evotown 拉取 Skill bundle 与 policy。

Evotown 负责控制面（账号、SkillHub、policy、网关）；Agent Doctor 负责员工电脑上的本地执行与审计。

## 职责划分

| 层级 | 产品 | 职责 |
|------|------|------|
| 控制面 | **Evotown** | 账号、Skill 市场、policy、批准 profile、审计接收、派活队列 |
| 本机客户端 | **Agent Doctor**（本仓库） | 发现 runtime、诊断、备份、修复、Skill sync、policy 缓存、**WS 在线与 inventory** |
| Runtime | OpenClaw、Hermes、Claude Code、Codex | 本地执行任务 |

## 员工 onboarding 流程

1. IT 部署 Evotown 并发放 `evk_` 员工密钥。
2. 员工安装 Agent Doctor（CLI 或桌面菜单栏）。
3. 连接 Evotown（任选其一）：
   - **桌面**：填写 Evotown URL + API Key →「连接、诊断并同步」
   - **CLI**：`agent-doctor setup --url https://your-evotown.example.com --key evk_...`
4. `agent-doctor doctor` — 查看已安装 runtime、网关 wiring、配置漂移。
5. `agent-doctor sync` — 从 Evotown SkillHub 安装/更新私有 Skill bundle。
6. `agent-doctor policy pull` — 缓存启用中的 policy 到本机。
7. 若 runtime 损坏或漂移：`agent-doctor repair <runtime> --apply`（备份 → 修复 → 复检 → 审计）。
8. **派活在线**：引擎 `register` 写入 `evi_` 后运行 `agent-doctor connect`（WebSocket presence + inventory + **执行 job.assign**）。

默认执行哪个本地 runtime：

1. 任务 `payload.runtime`（控制台可覆盖）
2. 否则本机 `EVOTOWN_RUNTIME`（`agent-doctor preferred-runtime use <id>`）
3. 否则按已安装优先：claude-code → codex → openclaw → hermes

`connect` 会上报 inventory 中的 `preferred_runtime`，Evotown 派活未指定 runtime 时默认填入该值。

协议说明见 Evotown [`docs/zh-CN/DOCTOR_NODE_PROTOCOL.md`](https://github.com/EXboys/evotown/blob/main/docs/zh-CN/DOCTOR_NODE_PROTOCOL.md)。

`connect` 收到任务后按 `payload.runtime` 执行：

| runtime | 行为 |
|---------|------|
| `claude-code` | `claude -p …` |
| `codex` | `codex exec …` |
| `openclaw` | POST `OPENCLAW_HOOK_URL` |
| `hermes` | POST `HERMES_HOOK_URL` |

## 配置文件

| 文件 | 写入方 | 用途 |
|------|--------|------|
| `~/.config/agent-doctor/profile.env` | `setup` | 公司 gateway URL、`evk_` key、runtime merge 源 |
| `~/.config/evotown/evotown.agent.env` | `setup` | Evotown 连接（与 legacy `evotown-agent-setup.py` 兼容） |
| `~/.config/evotown/skills-lock.json` | `sync` | 已安装 Skill 版本锁 |
| `~/.evotown/skills/` | `sync` | Skill 安装目录 |
| `~/.config/evotown/policies-cache.json` | `policy pull` | 本地 policy 缓存 |

`setup --url` 接受 Evotown **根 URL**（如 `https://evotown.example.com`）或完整 gateway URL（`.../api/gateway/v1`）。Agent Doctor 会自动规范化并写入各 runtime 配置。

## Evotown API

| API | Agent Doctor 命令 |
|-----|-------------------|
| `GET /health` | onboarding / `doctor` 连通性检查 |
| `GET /api/gateway/v1/health` | onboarding 网关检查 |
| `GET /api/v1/market/bundles/.../manifest` | `sync` |
| `GET /api/v1/market/skills/{id}/download` | `sync`（单 Skill 包） |
| `GET /api/v1/policies?enabled_only=true` | `policy pull` |
| `WS /api/v1/doctor/ws` | `connect`（presence + inventory） |
| `POST /api/v1/policy/evaluate` | 规划中（repair / ingest 前校验） |

## CLI 示例

```bash
# 1. 连接 Evotown（写入 profile.env + evotown.agent.env + runtime configs）
agent-doctor setup --url https://evotown.example.com --key evk_...

# 2. 诊断本机 runtime
agent-doctor doctor

# 3. 同步 Skill bundle（替代 evotown-agent-setup.py sync）
agent-doctor sync
agent-doctor sync --dry-run
agent-doctor sync --only my-skill --runtime openclaw

# 4. 拉取 policy 到本地缓存
agent-doctor policy pull

# 5. 常驻在线（需 EVOTOWN_ENGINE_ID + evi_ 已写入 evotown.agent.env）
agent-doctor connect
# 打开官方交互会话（Claude Code 优先 deep link，否则系统终端）
agent-doctor open claude-code
agent-doctor open claude-code --cwd /path/to/project -q "review recent changes"
```

## 替代 legacy 脚本

以下 Python 脚本能力已迁移到 Agent Doctor：

| Legacy | Agent Doctor |
|--------|----------------|
| `evotown-agent-setup.py check` | `setup` + `doctor` |
| `evotown-agent-setup.py sync` | `agent-doctor sync` |
| `evotown-agent-setup.py policy-pull` | `agent-doctor policy pull` |
| `evotown-agent-setup.py connector` | `agent-doctor connect`（在线 + 派活执行） |

仍保留在 Evotown 仓库的过渡能力：`register`、`watch`。`connect` 复用同一 `evi_` 做 WebSocket 鉴权与 job 结案。

## 企业 repair 保证

- 诊断本地优先，AI 分析前脱敏。
- 修复仅允许 typed、白名单 action。
- 写入前确认 + 备份快照。
- 审计报告含脱敏输入、选用 action、验证结果、回滚提示。
- Policy 可禁止原始日志/配置上传、破坏性 action、非公司 gateway。

Release 下载：`https://github.com/EXboys/agent-doctor/releases/latest`
