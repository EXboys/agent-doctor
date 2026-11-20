# exo 看板覆盖（中文 + Agent Doctor UX）

Agent Doctor **只要 exo 的分布式部署能力**，不要 exo 自带对话产品面。构建时会：

1. **`strings.tsv`**：中文文案覆盖（`英文<TAB>中文`）
2. **`apply_ux_patch.py`**：**只隐藏对话**（默认 topology-only，藏「返回对话」按钮）；组网引导、下载、对接方式等全部保留；顶栏增加「适合本机」
3. **`routes/suitable/`**：Agent Doctor 自建页——只展示当前能跑 / 腾内存后能跑的模型（不是 Downloads 全量目录）

注意：

- 只覆盖用户可见文案，不要写代码标识符（如 `DownloadCompleted`）。
- 长句写在短句前面更安全。
- 上游 Apache-2.0：产物带 `AGENT_DOCTOR_ZH_NOTICE.txt`。
- 构建缓存在 `~/.agent-doctor/cache/exo-dashboard/`（不入库）。

```bash
./scripts/build-exo-dashboard-zh.sh
# → ~/.agent-doctor/runtimes/exo/<PINNED>/dashboard-zh/
# 同步刷新内置包（打进桌面端，DMG 安装也能挂上定制看板）：
tar -czf vendor/exo-dashboard-zh/prebuilt/dashboard-zh.tar.gz \
  -C ~/.agent-doctor/runtimes/exo/<PINNED> dashboard-zh
```

客户端开启多机算力时会 `ensure_managed_dashboard_zh()`：没有 `dashboard-zh/` 就解压内置包，并用 `EXO_DASHBOARD_DIR` 挂载。点「打开算力看板」应看到「EXO算力管理」顶栏，而不是原版 exo 页。
