# ZCode 上游参考库（长期保留）

- 位置：`~/Documents/zcode-reference`（zai-org/ZCode 的源码快照，2026-11 克隆）。
- 用途：问答链路的交互/性能参考——折叠逻辑、时间线渲染、流式管线。
- 权利：Apache-2.0，允许借鉴；按 Paseo 同样的方式在文件头注明出处。

## 已借鉴并落地的部分

| 借鉴项 | 出处（参考库内） | 本仓库落点 |
| --- | --- | --- |
| 扁平时间线（文字段/工具按时间序交错） | `docs/chat/conversation-turn-view-model.md` | `desktop/src/chat/bubbles.ts`（seal=段边界）、`0795eb2` |
| 思考折叠/流式管线 | `docs/chat/reasoning-streaming-auto-collapse.md` | `desktop/src/chat/thinking.ts` |
| 工具分组摘要 | `docs/chat/execute-tool-call-grouping.md` | `desktop/src/chat/work-trail.ts`（“执行 N 条命令”） |
| 渲染性能预算 | `docs/chat/message-render-performance.md` | 32k 渲染上限、400 行挂载上限 |

## 待继续参考的主题

- turn 级虚拟化（`packages/ui/src/v4/ConversationTimeline.tsx` + 动态测高 + 前后 2 turn 预挂载）——若 400 行挂载上限后仍有长会话卡顿再上。
- 思考折叠态的单行横向跑马灯（动画队列 + 边缘 mask）——目前是 3 行滚动区，化妆级差异。
- 更新隔离契约（“每类消费者只订阅自己那块”）——做注册表收敛时可参考。

## 维护

- 需要 newer 代码时：`git -C ~/Documents/zcode-reference fetch --unshallow origin main`（当前是 depth-1 浅克隆）。
- 上游协议/行为变化对Ask 的影响，先看 `docs/chat/` 下同名主题文档，再看实现。
