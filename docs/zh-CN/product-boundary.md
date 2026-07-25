# 产品边界：个人（C）vs 团队（B）

Agent Doctor 是同一个本机客户端，共用运维内核。个人与团队是**模式**，不是两套产品。

## 共同内核

| 能力 | 含义 |
|------|------|
| **doctor** | 发现 runtime、探测配置/网关漂移、解释故障 |
| **repair** | 备份 → 类型化 playbook 修复 → 复检 → 审计（运维修好，不是「切换管家」） |
| **workspace** | 项目隔离，避免记忆 / MCP / Skill 串味 |

## 个人（C）

| | |
|--|--|
| **入口** | CLI + 轻桌面「本机修好」 |
| **价值** | 坏了能查、能修、项目不串味 |
| **增量** | 零配置就位、个人 provider、一键修好——仍是运维语义 |
| **不做** | 不做个人中转市场 / 用量看板产品 |

个人 Provider = 接线 endpoint + key + model，验证后写入 runtime，并修好 schema/网关。**服务商 URL/模型模板**（DeepSeek、OpenRouter 等）可以保留，那是填表便利，不是去拼中转生态。

## 团队（B）

| | |
|--|--|
| **入口** | 同客户端 + Evotown / 公司 profile |
| **价值** | 合规、基线、同步、派活、审计 |
| **增量** | `setup` / `sync` / `policy` / `connect` + 审计/合规导出 |
| **不做** | 不依赖个人中转生态做团队合规 |

Evotown 是控制面；Agent Doctor 仍是本机执行与修复工具。详见 [enterprise.md](../enterprise.md)。

## Profile 状态（勿串味）

| 文件 | 角色 |
|------|------|
| `~/.config/agent-doctor/profile.env` | **当前生效**的 runtime 覆盖层（个人或公司） |
| `~/.config/agent-doctor/company-profile.env` | **持久团队基线**（公司 `setup` 写入；个人激活不得覆盖） |
| `~/.config/agent-doctor/personal-providers.json` | 个人 Provider 列表 |
| `~/.config/evotown/evotown.agent.env` | Evotown 连接（URL / `evk_` / engine id） |

Workspace 的 **company baseline** 只对照 `company-profile.env`。激活个人 Provider 后，不得用个人 URL 去比对团队基线。

## 叙事约定

1. 默认叙事：本机运维 —— doctor / repair / workspace。
2. Evotown 是可选团队增量，不是唯一身份。
3. 个人配置文案用接线/修复语言（endpoint、验证、写入 runtime），不用市场语言（选套餐/选中转）。
4. Hermes 场景 `profile` 预设是场景切换便利，不是个人代理目录——范围限定在 Hermes 场景。
