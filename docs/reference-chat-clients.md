# 聊天体验机制：三个参考客户端对照表

体验类改动**先查这张表，再动手**。三个参考都是经过海量用户验证的实现；机制找不到现成做法时，明确标注「自研/实验性」，并给出探针数据的验收门槛。参考库与出处见文末。

参考实现：

- **Paseo** — `github.com/getpaseo/paseo`（Apache-2.0，web/Electron，TypeScript）。本仓库多个机制的原版出处。
- **Happy** — `github.com/slopus/happy`（iOS/Android 原生 + web）。其 Android 端 `research/telegram-chat.md` 是对 **Telegram Android**（commit 3de37c8c）的逐机制测量研究，即「海量用户验证」的一手材料。
- **ZCode** — `~/Documents/zcode-reference`（Apache-2.0，React + react-virtual）。另见 [reference-zcode.md](reference-zcode.md)。

## 长会话渲染（本次 16G Mac 卡顿的战场）

| 机制 | Paseo web | Happy/Telegram | ZCode | agent-doctor 现状 | 对齐状态 |
| --- | --- | --- | --- | --- | --- |
| 首开挂载量 | 最近 20 条（turn 对齐） | 15-25 条（高端机 25） | 可见 turn ±2 | 最近 80 行（turn 对齐，`MAX_MOUNTED_ROWS`） | ✅ 同量级 |
| 向上补页 | settle scheduler 分页 | 50 条/次 | 虚拟器自动 | settle 200ms 后 50 行/次（`loadEarlierPage`） | ✅ 常量对齐 Telegram |
| 补页触发带 | 滚动停止后 | 滚动中 25 行 / 静止 5 行 | 视口驱动 | 滚动停止且 `scrollTop<240px` | ⚠️ web 适配：行挂载是 markdown DOM（重），滚动中挂载会卡（beta.9 实测），故用 settle 门控。Telegram 能滚动中触发是因为文本布局离线预构建、绑定≈赋值——web DOM 做不到 |
| 单条超长 | 32k 字符渲染上限 + 折叠 | 900dp 截断（`PART_MAX_HEIGHT_DP`） | 虚拟窗口有界 | 32k 上限 + 「展开全文」 | ✅ |
| 工具/权限运行的边界 | `continuesTurn` 对齐，无上限 | 按条计数，无运行概念 | 虚拟器任意范围 | **run-slice 上限 48 行**（自研） | ⚠️ 自研但符合「每步渲染量有界」的三库共同原则：agent-doctor 需要运行边界做分组，三库都不需要 |
| 离屏行成本 | partial virtualization（阈值 100 条） | 回收池（恒定 DOM） | 虚拟窗口 | `content-visibility: auto` + 实测 `contain-intrinsic-block-size` + 行级 `contain: layout style`（自研，web 等价物） | ⚠️ WKWebView 对 flex 子项的 c-v 不可依赖——探针数据决定是否升 turn 级虚拟化 |
| 流式更新落点 | 头尾分离，尾帧只算尾轮 | 只更新变更的那一个 cell | 「冻结历史 + 尾轮」分区 | 头尾分离 + 围栏尾巴纯文本（本仓库已有，`bubbles.ts`） | ✅ |
| 贴底 | bottom anchor + stale-guard | reverse layout | following 状态 + 防「过期拽回底部」 | pin 状态（`gap<64`），滚动上滑即解除 | ✅ |
| 全量渲染路径 | **无** | **无**（超长走归档/回放） | **无** | **已移除**（`flushPendingHistory` 无界循环是 16G 卡 5 分钟的根因） | ✅ 根因已拔 |

## 本轮事故的流程沉淀（2026-11-12）

- v0.1.98-beta.9 的 Linux 构建在 `apt-get install` 上挂死 85 分钟 ×2：runner 镜像更新引入 conffile 交互提问，步骤无 `DEBIAN_FRONTEND` 无超时。修复：apt 步骤 noninteractive + 步骤级 `timeout-minutes`，桌面 job 级 45 分钟上限（`release.yml`/`ci.yml`）。处置流程按 [release.md](release.md) 重打标签执行。
- 三个参考实现都没有「一次渲染全部历史」的路径；agent-doctor 的 `flushPendingHistory` 无界循环是唯一例外，也是长会话卡 5 分钟的根因，已删除。
- **流程规则**：体验类改动先在这张表找到对应机制；三个参考做法不一致时，以 Telegram 测量研究的数据为准；找不到现成机制的，标注「自研/实验性」并给出探针验收门槛。

## 待办（由探针数据裁决）

- [ ] turn 级虚拟化（ZCode `ConversationTimeline` 形态：测高缓存 + spacer + 前后 2 turn 预挂载）——若 beta.10/11 实测滚动仍有持续 >50ms 长任务。
- [ ] 会话切换保留挂载 DOM（Paseo `retained-chat-content` 形态）——若切换长会话的重复挂载成本被用户感知。
- [ ] 流式更新的逐 cell 落点（Telegram 式「只更新变更 cell」）——若流式期间的 long task 归因到整列重排。
