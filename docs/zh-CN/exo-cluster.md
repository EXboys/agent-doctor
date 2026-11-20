# 多机算力集成设计（一期：个人版 + exo）

基于 [exo](https://github.com/exo-explore/exo)（Apache-2.0）为个人版客户端提供「装上客户端即可多台苹果电脑自动组网跑模型，Windows 同局域网直接连过去用」的能力。

本文档锁定一期的产品边界、运行时分发、组网隔离、Provider 双协议接线与界面文案。实现时严格遵守 [product-boundary.md](product-boundary.md) 与 [core-flows.md](core-flows.md) 的不变量。

---

## 一、产品边界与平台分工

### 1. 一期范围

| 维度 | 一期决策 |
|------|----------|
| **安装包门控** | 仅个人版（`AGENT_DOCTOR_EDITION=personal`）展示入口；团队版（`team`）隐藏，不混入 Evotown 合规网关链路 |
| **Apple Silicon Mac**（`aarch64-apple-darwin` 且 macOS ≥ 15.1） | **算力节点 + 消费端**：按需从 OSS 拉取锁死版本的无界面 `exo` 运行时，后台拉起并自动发现同组 Mac，完成模型切分摆放后写入本地个人 Provider |
| **Windows / Intel Mac / macOS < 15.1** | **纯消费端**：不下载、不运行推理引擎；自动扫描局域网 `52415` 端口寻找已开启的 Mac 算力集群，扫不到时支持填 IP 一键接入，写入同一个个人 Provider 槽位 |
| **接入形态** | 独立 `crates/agent-doctor-cluster`（`ClusterBackendKind::Exo`，预留 `Ollama` / `LlamaCpp`）+ `personal.rs` 内置 `local-cluster` Provider，**不做成 MCP 或对话内 Tool** |

### 2. 一期明确不做

1. **不把 `exo` 打进 Tauri 安装包 `resources/`**：避免主安装包膨胀数百 MB，也避免客户端每次日常更新都重下推理运行时。
2. **不做跨平台算力混编**：不在 Windows / Linux 上混跑 `llama.cpp RPC` 或 `exo` CPU 节点；异构设备在 `exo` 中本身无法握手组网。
3. **不自动改动系统雷电网络配置**：`exo` 的雷电桥接脚本（`io.exo.networksetup`）需要管理员密码并会切换 macOS 网络位置，可能导致用户固定 IP、DNS 或代理失效。一期只走零配置的局域网（Wi-Fi / 网线）与系统已有的直连网络。
4. **不做公网异地穿透**：组网限定在同一局域网内。

---

## 二、整体数据流

```text
  Apple Silicon Mac A                 Apple Silicon Mac B
 ┌─────────────────────────┐         ┌─────────────────────────┐
 │ Agent Doctor (个人版)    │         │ Agent Doctor (个人版)    │
 │ lifecycle/exo 按需下载   │         │ lifecycle/exo 按需下载   │
 │ 注入同一 NAMESPACE 拉起  │         │ 注入同一 NAMESPACE 拉起  │
 └────────────┬────────────┘         └────────────┬────────────┘
              │                                   │
              ▼                                   ▼
 ┌─────────────────────────┐  Zenoh  ┌─────────────────────────┐
 │ exo (0.0.0.0:52415)     │◄───────►│ exo (0.0.0.0:52415)     │
 │ MLX GPU 张量/流水并行    │ 自动组网 │ MLX GPU 张量/流水并行    │
 └─────┬─────────────▲─────┘         └─────────────────────────┘
       │             │
       │ 本机回环     │ 局域网扫描 :52415 / 填 IP
       ▼             │
  Mac 上的 Agent      └────────────── Windows / Intel Mac 上的 Agent Doctor
  (Claude Code / Codex / Hermes …)   写入 http://<mac-ip>:52415/v1 到个人 Provider
```

---

## 三、运行时打包与按需下载（`lifecycle/exo.rs`）

### 1. 运行时抽取与许可证合规

官方 `EXO.dmg` 内的 `EXO.app/Contents/Resources/exo/` 是一份已经过 Apple 代码签名的 PyInstaller 独立目录（入口可执行文件为 `exo/exo`），自带 Python、MLX 与 Rust 绑定，不依赖用户电脑安装 Xcode、`uv` 或 Python，也不会弹出 `EXO.app` 自己的菜单栏图标。

构建离线制品时：
1. 取固定版本的 `EXO.app/Contents/Resources/exo/` 目录。
2. 在归档根目录附带上游的 `LICENSE`（Apache-2.0）与 `NOTICE` 原文，满足 Apache-2.0 第 4 条分发要求。
3. 打包为 `exo-darwin-arm64-<PINNED_VERSION>.tar.gz`，计算 SHA-256。

### 2. 下载路由与版本锁死

`exo` 要求同一集群内所有节点的构建标识（`EXO_BUILD_ID`）与通信协议版本完全一致，否则节点会被隔离到 `/v1/devices-on-other-versions`。因此版本必须由客户端常量锁死，不允许执行浮动更新：

- 国内镜像源（`use_china_mirrors() == true`）：
  `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/runtimes/exo/<PINNED_VERSION>/exo-darwin-arm64.tar.gz`
- 海外回退源：GitHub Release 对应固定版本资产
- 本地落盘路径：`~/.agent-doctor/runtimes/exo/<PINNED_VERSION>/`
- 完整性校验：下载完成后校验 SHA-256，写出 `install.json`（含 `version`、`build_id`、`sha256`、`installed_at`）再原子替换目录；半截下载或校验不通过直接清理并报人话原因。
- 磁盘清理：用户点击卸载本机算力组件时，只删 `~/.agent-doctor/runtimes/exo/`，默认保留 `~/.exo/models/` 已下载的模型文件（除非用户明确勾选一并删除模型）。

> **升级门禁（对齐 `core-flows.md` 第五节第 7 条）**：修改 `PINNED_VERSION` 视为行为变更，升级前必须两台实机 Mac 验证组网握手、模型切分与对话回合完整收尾。

---

## 四、守护进程、权限与组网隔离（`crates/agent-doctor-core/src/exo/`）

### 1. 子进程规范与端口复用

1. **禁止裸调 `Command::new`**：统一通过 `crate::exec::command_for_path` 启动长驻进程，保持 `crates/agent-doctor-core/tests/raw_spawn_guard.rs` 围栏全绿；标准输出/错误重定向至 `~/.agent-doctor/logs/exo.log`（滚动截断）。
2. **启动前端口探测与孤儿进程回收**：记录受管进程 PID 文件；启动前先探测 `http://127.0.0.1:52415/node_id`：
   - 若端口未占用：由 Agent Doctor 拉起受管子进程，客户端退出或用户关闭「多机算力」开关时优雅发送 `SIGTERM` 回收；进程意外退出时最多自动重拉 1 次，再次失败则停下并用人话报错，禁止死循环重启。
   - 若已有外部 `exo` 在监听：检查状态并直接复用；若外部进程版本或组网名不匹配导致连不上其他电脑，在卡片上用一句人话说明「这台电脑上已经开着另一个版本的算力程序，关掉它再点开启才能和别的电脑连上」。
3. **运行期防休眠护栏**：多机切分推理期间任意一台 Mac 闲置休眠都会拖垮整条推理链。受管 `exo` 运行期间同步挂载生命周期绑定的防休眠进程（`caffeinate -i -w <exo_pid>`），`exo` 退出即自动释放，无需额外后台清理。

### 2. 组网隔离命名空间

当前已发布的 EXO.app（至 `v1.0.71`）仍走 **libp2p**，隔离变量是 `EXO_LIBP2P_NAMESPACE`。  
`main` 上已合并 libp2p→Zenoh 后，改为 `--namespace` / `EXO_ZENOH_NAMESPACE`。  
Agent Doctor 启动时会探测 CLI 是否支持 `--namespace`，并**同时注入两个环境变量**，同一套账号/口令命名空间在新旧运行时都能隔离。

**关键避坑**：`teamups_auth.rs` 中每次设备扫码登录拿到的 `license` 是设备级凭据，两台登录同一账号的 Mac 各自持有不同的 `license`，**不能对 `license` 做哈希当集群名**。

命名空间按以下优先级生成，确保「自己的电脑自动连，旁人的电脑进不来」：

1. **手动组网口令优先**：若用户在卡片「详情」里设置了自定义组网口令（存 `settings.db` 的 `exo.cluster_code`，如 `8842`），使用 `ad-code-<口令>`。
2. **同账号自动对齐**：已登录 TeamUps 时，在 `crates/agent-doctor-core/src/skills/teamups_auth.rs` 请求 `/api/v1/doctor/me` 返回体中提取稳定的账号标识（优先读 `id` / `userId`，无则读规范化后的账号 `name`，随 `ACCOUNT_STATUS_CACHE_KEY` 一起持久化到 `settings.db`），生成 `ad-acct-<sha256(base_url + ":" + account_key)[..12]>`。同一账号下的多台 Mac 无需填任何口令，打开开关即在同一命名空间。
3. **未登录随机口令兜底**：未登录 TeamUps 且未填口令时，首次开启自动生成一个 4 位数字口令存入 `exo.cluster_code` 并显示在卡片副标题（「另一台电脑填口令 `4921` 即可加入」），绝不落回默认全局 `exo` 名字，防止在办公室或咖啡馆 Wi-Fi 下与陌生人的电脑自动拼成一团。

### 3. 启动环境变量表

| 环境变量 | 值 | 作用 |
|----------|----|------|
| `EXO_RUNTIME_DIR` | `~/.agent-doctor/runtimes/exo/<ver>/exo` | 指定 PyInstaller 运行时根目录 |
| `PATH` | `<EXO_RUNTIME_DIR>:$PATH` | 确保内部工具链优先命中内嵌二进制 |
| `EXO_BUILD_ID` | `<PINNED_EXO_BUILD_ID>` | 锁定构建号，保证集群内多台 Mac 握手一致 |
| `EXO_LIBP2P_NAMESPACE` | `ad-acct-...` 或 `ad-code-...` | libp2p 发布版（≤1.0.71）集群隔离 |
| `EXO_ZENOH_NAMESPACE` | 同上 | Zenoh 新构建集群隔离（与上并存注入） |
| `EXO_DASHBOARD_DIR` | `.agent-doctor/runtimes/exo/<ver>/dashboard-zh` | 指向汉化后的静态看板目录（免重编二进制，不破坏 Apple 代码签名） |
| `HF_ENDPOINT` | `https://hf-mirror.com`（仅 `use_china_mirrors()` 为真时注入） | 解决中国大陆拉取 HuggingFace MLX 模型超时卡死 |

### 5. 自带管理看板免重编汉化与状态同步

`exo` 服务端通过 `FastAPI StaticFiles` 将看板挂载在根路径 `http://<host>:52415/`，并在 `exo/shared/constants.py` 中原生读取环境变量 `EXO_DASHBOARD_DIR`（相对于 `$HOME` 解析，即 `Path.home() / os.environ["EXO_DASHBOARD_DIR"]`）：

1. **本地一键构建中文静态包**（不重编签名二进制）：
   ```bash
   ./scripts/build-exo-dashboard-zh.sh
   # 产物安装到 ~/.agent-doctor/runtimes/exo/<PINNED>/dashboard-zh/
   ```
   - 文案覆盖（入库）：`vendor/exo-dashboard-zh/strings.tsv`（只替换引号字符串与标签文本，避免误伤 `ChatForm` 这类标识符）
   - **UX 裁剪（入库）**：`vendor/exo-dashboard-zh/apply_ux_patch.py` + `components/HeaderNav.svelte` — **只隐藏 exo 对话**（默认 topology-only，藏返回对话按钮）；页眉重做为主窗口同款蓝青徽标 +「EXO算力管理」+ 胶囊分段切换（适合本机 / 下载 / 设备）
   - **视觉对齐**：`vendor/exo-dashboard-zh/agent-doctor-theme.css` 去掉上游科幻扫描线、流星与等宽大写工业风，改为主窗口同款浅灰蓝画布、大圆角白卡片与系统无衬线字体；窗口标题栏与主窗口相同（macOS 红绿灯 / Windows 自绘最小化、最大化、关闭）
   - **构建缓存不入库**：上游 checkout / `node_modules` 放在 `~/.agent-doctor/cache/exo-dashboard/<ver>/`，避免仓库内嵌套 `exo/.git` 误触发 Cursor/git 工作区识别
   - 上游 tag 缺失时脚本会退回 `main` 仅供开发；正式 OSS 包必须锁到与运行时一致的 ref
2. **双端入口与唯一事实来源（防状态打架）**：
   - 桌面卡片提供次级入口**「打开算力看板」**：内嵌 WebView 先打开 `http://<host>:52415/`，再在页面内切到 `#/suitable`（**适合本机**）。不要把 hash 写进加载 URL——WKWebView 在较新 macOS 上会把 `#/suitable` 当成路径去请求 `/suitable`，FastAPI 直接返回 `404 Not Found`。无 Suitable 覆盖时会退到 `#/downloads`；顶栏仍可切下载与设备拓扑
   - 若用户在看板里手动切换或删除了模型实例，客户端每次刷新读取 `GET /state` 同步回 `local-cluster` Provider

本地试跑步骤：

1. `cd desktop && npm run tauri:dev:personal`
2. （开发机）`./scripts/build-exo-dashboard-zh.sh` 生成中文看板到 `~/.agent-doctor/runtimes/exo/<PINNED>/dashboard-zh/`
3. 打开「模型服务」→「多机算力」→「下载并开启」（首次拉官方 `EXO-latest.dmg`；OSS 包尚未上传时走此回退）。**重新安装会保留已有 `dashboard-zh/`**
4. 点「打开算力看板」：应在桌面侧窗打开「适合本机」；顶栏有「适合本机 / 下载」；日志里应出现 `EXO_LIBP2P_NAMESPACE: ad-…`（1.0.71）

### 4. macOS 本地网络权限

由 Agent Doctor 直接拉起内嵌 `exo` 子进程时，macOS Sequoia（15.1+）按父应用校核本地网络 TCC 权限。需在 `desktop/src-tauri/Info.plist` 增加：

```xml
<key>NSLocalNetworkUsageDescription</key>
<string>需要查找同一网络里的其他苹果电脑，才能把几台电脑连起来一起跑模型。</string>
```

当用户拒绝权限导致只能看到本机 1 台设备时，在「连不上其他电脑？」折叠提示里给出直白引导：「打开系统设置 → 隐私与安全性 → 本地网络，把 Agent Doctor 打开后再试一次」。

同时轮询 `GET /v1/devices-on-other-versions`：若发现同名集群下存在版本不同的设备，直接提示「发现另一台电脑的客户端版本不一致，把两台都更新到最新版就能连上」。

---

## 五、模型实例编排与双协议 Provider 接线

### 1. 模型自动摆放流水线（`exo/orchestrator.rs`）

`exo` 不会在收到 `/v1/chat/completions` 时自动创建实例，必须由 Rust 侧在启用或切换模型时跑完摆放闭环：

1. **查现成实例**：调 `GET /state`，若当前集群中已有目标 `model_id` 的就绪实例，直接复用。
2. **按集群总内存推荐默认模型**：小白不认识 `mlx-community/...` 模型名。客户端汇总 `GET /state` 中所有在线节点可用统一内存，默认给三档人话选项（真实 HuggingFace ID 放详情）：
   - **轻量快速**（总可用内存 ≥ 8 GB）：Qwen 2.5 / Qwen 3 7B~8B 4-bit
   - **日常主力**（总可用内存 ≥ 24 GB）：Qwen 3 30B-A3B / 32B 4-bit
   - **多机大模型**（总可用内存 ≥ 64 GB，多台 Mac 合体后解锁）：Qwen 3 / DeepSeek 大参数 4-bit
3. **预览并提交切分（预留系统内存水位）**：
   - `GET /instance/previews?model_id=<model_id>`，挑选第一条 `error == null` 且各节点剩余系统内存仍留有安全余量（≥ 4 GB）的摆放方案，绝不吃满用户电脑统一内存导致系统卡死。
   - 若所有预览均报错（内存不足），立刻终止并返回人话：「这几台电脑加起来可用内存不够放这个模型，换小一档模型或再开一台电脑加入」。
   - `POST /instance` 提交选中的 `instance` 对象。
4. **等待就绪**：连接 SSE 流 `GET /instance/await?model_id=<model_id>&timeout_seconds=300`，前端卡片实时显示「正在准备模型（首次使用需要下载）…」，收到 `{"type": "ready"}` 后进入下一步接线。

### 2. 一次写入同时打通 OpenAI 与 Claude Code（核心接线点）

`exo` 在同一端口 `52415` 上原生同时提供：
- OpenAI 协议：`POST /v1/chat/completions`、`POST /v1/responses`（base URL 带 `/v1`）
- Anthropic 协议：`POST /v1/messages`（Claude Code 会自己拼接 `/v1/messages`，因此 `ANTHROPIC_BASE_URL` **不带 `/v1`**，与现有硅基流动映射规则完全一致）

改动点只需两处即可复用整套 `apply_mode_switch`：

1. **`crates/agent-doctor-core/src/setup/pipeline.rs`**：
   在 `dual_protocol_endpoints(url)` 中识别 `exo` 端点（匹配端口 `:52415` 或由内置 `exo-cluster` 条目传入的地址 `http://<host>:52415/v1`），返回：
   - `openai_url`: `http://<host>:52415/v1`（自动投影到 Codex、Hermes、OpenClaw、DeepSeek Harness 等）
   - `anthropic_url`: `http://<host>:52415`（自动投影到 Claude Code）
2. **`crates/agent-doctor-core/src/setup/personal.rs`**：
   - 新增保留 ID 常量：`pub const EXO_CLUSTER_PROVIDER_ID: &str = "exo-cluster";`
   - **占位密钥**：`load_active_personal_provider()` 与 `load_personal_provider()` 会过滤 `api_key.trim().is_empty()` 的条目。因此写入 `exo-cluster` 时固定填入 `api_key: "exo-local"`（`exo` 服务端忽略 Bearer 头，不会报错，同时保证现有所有非空校验与故障切换链路无需特判）。
   - 激活后触发跨窗口状态广播，保证主页与问答窗口顶部的模型标签立即同步（对齐 `core-flows.md` 第六节）。

---

## 六、Windows（及非 Apple Silicon Mac）局域网发现与消费接入

在不支持运行 `exo` 的机器上，客户端作为纯消费端接入局域网已有的 Mac 集群：

### 1. 自动扫描（`exo/lan_scan.rs`）

用户点击「查找局域网里的苹果电脑算力」时：
1. 获取本机所有活动非回环 IPv4 网卡地址（跳过 `169.254.x.x` 链路本地地址及常见容器虚拟网段），取对应 `/24` 子网。
2. 在后台任务中以信号量限制并发度（`32` 路并发），对每个 IP 的 `52415` 端口发起 `350ms` 超时的 TCP 探测。
3. TCP 连通的地址继续请求 `GET http://<ip>:52415/node_id` 与 `GET http://<ip>:52415/state`（超时 `800ms`）：
   - 校验返回合法 JSON，汇总该集群内有几台 Mac、当前已就绪的 `model_id` 列表。
   - 按集群 `session_id` / 节点集合去重（同一集群的 3 台 Mac 都会响应 `52415`，界面合并展示为 1 个集群，优先选延迟最低或当前担任 master 的节点 IP）。

### 2. 手动填 IP 兜底

若公司网隔离了有线与无线网段导致扫描未命中，界面直接展开单行输入框：
- 用户只需填任意一台已开启算力的 Mac 局域网 IP（如 `192.168.1.23`，也兼容粘贴完整 URL），客户端自动规范化为 `http://192.168.1.23:52415/v1`。
- 若远端集群尚未加载模型，消费端同样可以通过该节点的 REST API（`/instance/previews` → `/instance` → `/instance/await`）触发加载并等待就绪，随后激活 `exo-cluster` Provider。

### 3. 断线与故障切换

当局域网里的 Mac 盒盖休眠或关机时，请求返回连接拒绝或超时，`provider_failover.rs` 原生将其归类为 `Outage::Down`（冷却 60 秒）：
- 若用户还保存了「官方」或其他个人服务商，当前对话会自动临时切到备选服务商，并用一句人话告知「刚才那台本地算力电脑连不上，这次改用 X」。
- 若没有其他服务商，错误提示按人话展示：「连不上提供算力的苹果电脑，请确认那台电脑开着且在同一个网络里」。

---

## 七、桌面端界面与小白文案规范（个人版）

入口位于个人版的「模型服务」区域（`desktop/src/wiring/personal.ts`），作为独立于普通 API Key 表单的**「多机算力（局域网免费跑模型）」**卡片。所有文案在 `desktop/src/i18n/zh.ts` 与 `en.ts` 成对注册，通过 `desktop/tests/i18n-pairs.test.ts`。

### 1. Apple Silicon Mac 界面状态与按钮

| 当前状态 | 一句话说明（用户可见） | 按钮文案（写点击后的变化） |
|----------|------------------------|----------------------------|
| 未下载组件 | 把同一网络里的几台苹果电脑连起来，一起免费跑本地模型。 | **下载并开启多机算力** |
| 下载中 | 正在下载本机算力组件（约 X MB）… | **取消下载** |
| 已启动（仅本机 1 台） | 已开启：当前 1 台电脑在线。先选档位，设为对话模型后到问答里直接问。 | **设为对话模型**（打开问答） |
| 多机合体（N ≥ 2 台） | 已连上 N 台苹果电脑（合计可用内存 X GB），正在一起分担模型。 | **用这 N 台电脑对话** / **换模型档位** |
| 模型准备中 | 正在把模型放到这 N 台电脑上（首次使用需要下载模型）… | 禁用态显示进度 |
| 发现版本不一致设备 | 发现同一网络里有另一台电脑，但客户端版本不一致，暂时没连上。 | **检查更新** |

### 2. Windows / Intel Mac 界面状态与按钮

| 当前状态 | 一句话说明（用户可见） | 按钮文案（写点击后的变化） |
|----------|------------------------|----------------------------|
| 初始未连接 | 如果同一网络里有开启了算力的苹果电脑，这台电脑可以直接连过去用。 | **查找网络里的算力电脑** |
| 扫描命中 | 找到 N 台苹果电脑组成的算力（可用内存 X GB）。 | **连上并用它对话** |
| 扫描未命中 | 没在当前网段找到算力电脑。先在苹果电脑上打开「多机算力」，或在下面填它的局域网地址。 | 输入框 + **连上这台电脑** |

> 端口 `52415`、组网口令/命名空间、真实 HuggingFace 模型 ID 与日志路径统一收纳在折叠的「详情」里，不作为唯一说明。

---

## 八、分步实施计划与验收看护

确认本设计后，代码按 4 个独立可测的步骤落地：

1. **Step 1 — 双协议映射与内置 Provider 槽位（纯 Rust 核心 + 单测）**
   - 修改 `crates/agent-doctor-core/src/setup/pipeline.rs`：为 `:52415` 补齐 `dual_protocol_endpoints`（OpenAI `/v1` + Anthropic 根路径）。
   - 修改 `crates/agent-doctor-core/src/setup/personal.rs`：新增 `EXO_CLUSTER_PROVIDER_ID` 与 `upsert_exo_cluster_provider`（自动写入 `"exo-local"` 占位密钥并激活）。
   - 修改 `crates/agent-doctor-core/src/skills/teamups_auth.rs`：缓存稳定的账号标识供命名空间派生。
   - **看护**：`pipeline.rs` 双协议映射单测 + `merge/` 各 runtime 投影单测。

2. **Step 2 — `exo` 按需生命周期、守护进程与 REST 编排器**
   - 新增 `crates/agent-doctor-core/src/lifecycle/exo.rs`：OSS / 官方双路由下载、SHA-256 校验、解压与清理。
   - 新增 `crates/agent-doctor-core/src/exo/`：`daemon.rs`（经 `command_for_path` 启停、注入 `EXO_BUILD_ID` / `EXO_ZENOH_NAMESPACE` / `HF_ENDPOINT`）、`orchestrator.rs`（`/state`、`/instance/previews`、`/instance`、`/instance/await` SSE 等待）、`lan_scan.rs`（Windows 消费端 `/24` 并发扫描）。
   - 修改 `desktop/src-tauri/Info.plist`：补充 `NSLocalNetworkUsageDescription`。
   - **看护**：`cargo test -p agent-doctor-core --test raw_spawn_guard` 必须通过；编排器与扫描器 Mock HTTP 单测。

3. **Step 3 — Tauri IPC 命令与跨窗口广播**
   - 新增 `desktop/src-tauri/src/commands/exo.rs` 并在 `lib.rs` 注册命令（查状态、安装并开启、停止、选档位摆放并设为当前 Provider、局域网扫描、填 IP 连接）。
   - 状态变更与 Provider 切换发出全局窗口事件；应用退出钩子回收受管 `exo` 子进程。

4. **Step 4 — 个人版桌面 UI 与中英文文案**
   - 在 `desktop/src/wiring/personal.ts`（仅 `personal` 包）接入「多机算力」卡片，区分 Apple Silicon 节点视图与 Windows 消费端视图。
   - 在 `desktop/src/i18n/zh.ts` 与 `desktop/src/i18n/en.ts` 成对添加文案。
   - **看护**：`desktop/tests/i18n-pairs.test.ts` 全绿；遵循开发环境规范通过 `npm run tauri:dev:personal` 在主页与问答双窗口实机验证。
