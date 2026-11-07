# Windows 子进程与外部 runtime 环境

这一页管两类「在 mac 上写代码时永远看不见」的事故：Windows 子进程的控制台/窗口行为，和把环境交到外部 agent（Claude Code / Codex / dsh / Hermes / OpenClaw…）手里之前要兜住的底。规则全部来自真实事故，括号里是修复提交。改子进程、改安装器、升级 pin 的 runtime 版本之前，先读这一页；总纲见 [core-flows.md](core-flows.md) 第五节。

## 一、进程创建标志：只有三种，各有唯一住所

| 标志 | 语义 | 唯一合法住所 | 用途 |
| --- | --- | --- | --- |
| `CREATE_NO_WINDOW` (0x08000000) | 给子进程一个**不显示窗口的控制台，整棵子孙树继承它** | 其他一切地方 | 探测、安装、GUI 应用下起的一切控制台子进程 |
| `DETACHED_PROCESS` (0x00000008) | 子进程**没有控制台**；它再起的控制台程序各自新开**可见**窗口 | `exec/process.rs`（长驻 CLI 会话） | 会话进程就是终点、不指望它再起别的东西 |
| `CREATE_NEW_CONSOLE` | 给子进程一个**新的可见**控制台 | `session_launch/terminal.rs` | 有意给用户开终端 |

一句话记住区别：**`DETACHED_PROCESS` 不是「隐藏窗口」，是「没有控制台」——隐藏要靠 `CREATE_NO_WINDOW`。** 判断方法：这个子进程自己会不会再起别的控制台程序？会（cmd → npm → node、cmd → powershell → git）→ `CREATE_NO_WINDOW`；它自己就是终点 → 两个都能用，选 `CREATE_NO_WINDOW` 不会错；想给用户看 → `CREATE_NEW_CONSOLE`。

**事故史：**

- 0.1.87–0.1.90：裸 `Command::new` 闪窗四连 → `8cdcd00` 建立第一条围栏（裸 spawn 必须走 `exec` 或带 `// raw-spawn-ok` 标注）。
- `a678e8b`（2026-10-19）：为修「启动探测闪窗」，把安装 runner 的 `CREATE_NO_WINDOW` 一并换成了 `DETACHED_PROCESS`。探测修好了，但从这半天起，**每装一个 runtime 都弹一个 powershell 黑窗**（Claude Code 的 npm→node.exe、DeepSeek/Hermes 的 powershell、Hermes 装机下载的 git/uv 全中），潜伏三周后被用户发现 → `88c81ab` 恢复。**当时的围栏拦不住它：`raw_spawn_guard` 只验「spawn 有没有标注」，不验「标志选得对不对」——runner.rs 恰好是有标注的。** 这就是第二条围栏（标志住所）存在的原因。

**不变量：**

1. 每个spawn 的标志按「它的孩子还要起什么」选，不按「和别处保持一致」选。
2. 标志只许住在唯一合法文件里：`DETACHED_PROCESS` 只在 `exec/process.rs`，`CREATE_NEW_CONSOLE` 只在 `session_launch/terminal.rs`。围栏 `creation_flags_stay_in_their_sanctioned_files`（`raw_spawn_guard.rs`）拦搬家；`runner.rs` 的 `install_children_share_one_hidden_console` 在 CI `rust-windows` 里钉死安装标志的值。
3. **修哪条路只动哪条路。** 一次修复顺手「统一」另一处的标志，就是 `a678e8b` 的做法。动任何 spawn 点前，先说出这个进程的孩子树：它起谁、那些孩子又起谁。

## 二、交给外部 runtime 的环境：由我们清洗，坏值说人话

外部 agent 是黑盒 npm 包，对宿主环境有不设防的假设，坏了就把原始 Node 错误直接抛进对话。这些炸法在 mac 上永远复现不了，所以约定：**环境我们清洗后交付；拦不下的问题必须翻译成人话并给出下一步。**

启动 dsh 这类 Node 系 runtime 前（`prompt_session/deepseek_harness.rs`）：

1. **TEMP/TMP 清洗。** 缺失、不是绝对路径、不存在、或指向盘符根（`C:` / `C:\`）→ 换成 `%LOCALAPPDATA%\Temp`。dsh 的 Windows 沙箱**每轮对话**都对 `os.tmpdir()` 做一次无保护的 `realpathSync`；TEMP 指向盘符根时就是每轮 `Internal error: turn failed: EISDIR: illegal operation on a directory, realpath 'C:'`（Node/libuv 十年未修的 [nodejs/node#6861](https://github.com/nodejs/node/issues/6861)）。（`88c81ab`）
2. **会话目录不许是盘符根。** 拦下并说人话（「DeepSeek 不能把整个磁盘当项目目录……请选一个具体的文件夹」），不甩原始错误。（`88c81ab`）
3. **剥掉 verbatim 前缀。** Rust 的 `fs::canonicalize` 在 Windows 返回 `\\?\C:\…` 拼写；交给外部 runtime 前剥成普通 Win32 拼写，别让 verbatim 进它们的会话记录和路径比较。（`88c81ab`）

**不变量：**

4. 清洗逻辑写成纯函数并配测试：盘符根拼写、verbatim 剥离、TEMP 判定三组测试看护；Windows-only 的那组（TEMP 交换）在 CI `rust-windows` 里跑。本机是 mac 时，Windows 行为以 `rust-windows` 绿为准。
5. 拦截类失败（盘符根目录、CLI 没装、版本太旧）的一句话必须回答「我该改什么、去哪点」，不许只复述错误码。

## 三、升级 pin 的外部 runtime = 行为变更，不是刷版本号

dsh `0.1.0-rc.6 → 0.1.5-rc.3`（`8e4156e`，为了修「旧版没有聊天 profile、对话起不来」）让 DeepSeek 聊天第一次真正跑起来——同时也第一次走进了 dsh 沙箱里那段无保护 realpath。EISDIR 的暴露面就是这次升级带进来的：旧版走不到那一步，所以「之前没有这个问题」。

**不变量：**

6. 升级 pin 版本前，把旧版装下来对比：`npm install <pkg>@<旧版> --prefix /tmp/x`，比包树（新出现哪些子包）和关键模块的差异；升级说明写清「这个版本给我们带来了什么、可能改变什么」。
7. 升级后在 Windows 实机跑一轮真实对话再算升级完成。CI 之外，这一步没有替代品。

## 看护

- `crates/agent-doctor-core/tests/raw_spawn_guard.rs`：`raw_command_spawn_is_confined_or_marked`（裸 spawn）、`creation_flags_stay_in_their_sanctioned_files`（标志住所，本页第一节的不变量 2）。
- `crates/agent-doctor-core/src/lifecycle/runner.rs`：`install_children_share_one_hidden_console`（安装标志钉值，rust-windows 跑）。
- `crates/agent-doctor-prompt/src/prompt_session/deepseek_harness.rs`：`drive_root_spellings_are_caught_and_normal_folders_pass`、`acp_cwd_gives_dsh_plain_windows_spellings`（跨平台可跑）；`broken_child_temp_values_are_swapped_healthy_ones_kept`（rust-windows 跑）。

## 改动清单

- [ ] 碰任何 spawn 点：先说出这个进程的孩子树，标志按第一节表格选
- [ ] 碰外部 runtime 启动环境：按第二节 1–3 清洗，拦截失败带下一步
- [ ] 升级 pin 的 runtime：先 diff 旧包，升级后在 Windows 实机跑一轮对话
- [ ] `make lint`（跑两条围栏）+ 等 CI `rust-windows` 绿了再合
