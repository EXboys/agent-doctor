# 助手子进程的网络代理

`agent-doctor-proxy` 决定 Claude Code、Codex、反重力、Hermes 等子进程该用哪个 HTTP 代理。它**不是**代理服务器，不转发任何流量。

从程序坞打开的桌面 App 只有一套最小环境，拿不到 `~/.zshrc` 里的 `https_proxy`。同一条命令在终端里能通，在 App 里就会对海外模型主机超时。这个 crate 补上这一环。

## 优先级（先命中的生效）

1. `AGENT_DOCTOR_PROXY` — 手动指定的地址；填 `direct` / `off` / `none` 则去掉所有代理。
2. 当前进程里已有的代理变量（从终端启动 App 时）。
3. 用户登录 shell 里的代理变量，且本机代理端口能在约 300 ms 内应答。
4. 系统代理（macOS 网络设置 / Windows Internet Settings），且端口能应答。
5. 什么都不做。TUN / VPN / OpenVPN 用户落在这一档。

从 shell 里只取代理和证书相关变量，不会把 `ANTHROPIC_API_KEY` 之类带进子进程。

## 接入点

- 桌面端启动时调用 `agent_doctor_core::proxy::warm()`。后台立刻算一份计划放进内存，之后每 30 秒重读系统代理并试连本机端口。登录 shell 只读一次。
- 启动子进程时只读这份内存，不等 shell、不连端口。`AGENT_DOCTOR_PROXY` 和进程里已有的代理变量仍当场生效。
- `exec::command_for_path` 统一套到所有助手子进程。必须直连的调用方（国内安装下载）在建好命令后再删掉代理变量，与现有写法一致。
- `raw_spawn_guard` 也检查这个 crate 的源码。

## 边界

- 只开了 SOCKS 的系统代理不会被套上：Claude Code 和 Node 不认 SOCKS。请用 HTTP 端口（Clash 混合端口、V2rayU 的 HTTP）。
- PAC 自动配置换不出单一地址，保持直连并记一条说明。
- 走国内镜像的安装脚本仍走自己的路径，不受这套计划强制。

## 失败文案

传输层失败（重连耗尽、通道关闭、没有 HTTP 状态的超时）归为 `network`，不是 `provider_url`。界面说连不上模型服务器，并提供「再问一次」，不会说「去核对地址」。
