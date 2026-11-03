# 核心链路：改动前后的标准

这是五条「坏了用户就用不了」的链路的改动标准。这里的规则全部来自真实事故（每条后面括号里是对应的修复提交）。改这些链路时先看这一页，改完跑最后一节的检查。不要为某一次事故加一次性分支——把规则落到下面的不变量里。

## 一、问答回合（一轮的生命周期）

一轮 = 用户发出一句话到这轮收尾的整个过程。

**不变量：**

1. **一轮必须恰好收尾一次。** 回复完成、失败、用户停止、后台工作回报、自动允许没通过——每一种都要走到收尾。收尾过一次就不再收第二次。（63e2335、2328356）
2. **慢不是死。** 不能靠固定超时判定回合已死然后拆掉它；只有 runtime 明确报结束/失败/停止才收尾。（762a499）
3. **被顶替的回合不做收尾清理。** 这次 invoke 返回时如果已经有更新的一代在跑，按「started」返回，不清映射、不清状态、不回渠道。（chat/send.ts）
4. **被限流要说「等」，不能原样重发。** 重发等于把用户的额度再烧一次。（c951052）
5. **续接不能拿别的 runtime 的 session id。** 每个runtime 的 thread/session 各自匹配。（39b5181）
6. **计时从各自的起点算。** thinking 时长从 thinking 开始算，不是从整轮开始算。（a1b3cf1）

**看护：** `desktop/tests/turn-clock.test.ts`、`chat-failure.test.ts`、`send-round.test.ts`、`channel-ask.test.ts`（映射存活）。

## 二、电脑 ↔ 手机通道

手机（飞书）把一句话送到这台电脑，电脑把回复送回手机。

**不变量：**

1. **回复回到提出这句话的那个 chat**，不是当前屏幕上的 chat。job 的 channel key 在接活时记下，答复时带上。（b722473）
2. **job→session 映射活到这轮再也无法取消为止。** 提前清掉，手机按停止就找不到该停谁。（chat/send.ts、`channel-ask.test.ts`）
3. **停止必须真正到达任务；重复停止幂等。** 手机在界面安静时会反复发停止，一次生效就够。（7382c44）
4. **找不到任务时不许撒谎。** 不能替别的电脑宣布「停了」，也不能替还活着的回合关掉任务；账本里有答案就补发答案，没有就明说「这台电脑上没找到」并保持任务开放。（channel-stop）
5. **每句话恰好答复一次。** 断线重发、应用重启，都由账本（channel-ledger）兜底；跑一半关机的句子重启后不重跑。（`channel-ledger.test.ts`）
6. **手机列表带上下文。** 项目、状态、步骤跟着发，手机上才看得懂进度。（65ee48b、fdc4c94）

**看护：** `desktop/tests/channel-stop.test.ts`、`channel-ask.test.ts`、`channel-ledger.test.ts`、`channel-session.test.ts`。停止的判定表在 `desktop/src/chat/channel-stop.ts`，是纯函数——改停止行为先改判定表，再让 chat-boot 接线。

## 三、配置与密钥

setup / 模型服务页 / 修复，都会写用户的配置文件和密钥。

**不变量：**

1. **写配置是合并，不是覆盖。** 用户文件里和我们无关的段落必须原样保留。丢一次 key，用户就要重新填一次表。（5b5e5b3、2767657、f160e92——同一个 bug 犯了三次）
2. **已保存的 key 必须活得过这些动作：** 订阅登录、能力探测、版本检查、浏览器工具检测。任何一条路径里「读到空值就当没有」都会把 key 洗掉。（同上）
3. **凭据写失败必须让用户看见。** 静默失败 = 用户以为存上了。（a3b5243）
4. **文案 key en/zh 成对出现。** 缺一边就是用户界面上的一块空白。（993fa71，`i18n-pairs.test.ts` 看护）
5. **端点按服务商映射。** 换 base URL 时同步换 API 形态（如 Zhipu GLM → Codex Responses），不要只改域名。（dd567b6、3ee0cfa）

**看护：** `crates/agent-doctor-core/src/setup/merge/` 的单元测试、`desktop/tests/model-picker-subscription.test.ts`、`i18n-pairs.test.ts`。

## 四、下载与安装

装 runtime、装 CLI、装模型服务客户端。

**不变量：**

1. **安装不允许被墙卡死。** 大陆不可达的官方源要能跳过并说明原因，走可达的镜像；跳过的要写进结果。（1adc3dc）
2. **Windows 上一律经共享 runner 起子进程**（`lifecycle/runner.rs`、`exec/process.rs`），禁止裸 `Command::new`——每个裸调用都是用户桌面上的一个黑窗。（8cdcd00、a678e8b、277836e、9507226、9e8d193——修了五轮）
3. **PATH 只做尾部增量合并，不重排。** Windows PATH 超长时降级保可达，不能把应用或 Claude/Node 搞挂。（74a5694、26394a9、2a5ff8e）
4. **每个安装步骤有进度、有失败原因。** 用户不该面对一个静默消失的安装。（8e4156e）

**看护：** CI 的 `rust-windows` job（这是 Windows-only 代码唯一的编译+lint+测试处）；涉及子进程的改动在 Windows 上必须实机看过一次。

## 五、官网（site/）

静态站，Cloudflare Pages 托管，下载走 GitHub Releases / OSS。

**不变量：**

1. **改了 `site/css/site.css` 或任何 `site/js/*.js`，部署前必须重跑 `scripts/site-version.sh`。** 边缘缓存无视 `_headers`，不带新 `?v=` 的部署会让用户拿到新旧混搭的页面，长达数小时。（bad0ba3；`scripts/check-site.mjs` 看护）
2. **站内链接和 `_redirects` 必须指得到。** 删页、改名之后，导航、锚点、跳转规则要跟着改。（`check-site.mjs` 查页面存在和锚点存在）
3. **手机安装包在 OSS 默认域名下不能用 `.apk` 后缀存放**（默认域名禁 apk 下载），改用无后缀存放并让下载链接指向它。（b843f37、30bd16b）
4. **发布产物不能是草稿状态**，上传收尾后要校验链接真的可达。（73d9685、4a038b0——工作流变量名和站点更新脱节）

**看护：** `node scripts/check-site.mjs`（本地 `./scripts/check.sh site`、CI `site` job）。第 3、4 条需要网络和 OSS 凭据，发布工作流自己校验，人盯最后一步。

## 改完必须跑

```bash
make lint                                   # fmt + clippy（git 钩子也会拦）
./scripts/check.sh site                     # 官网静态检查
cd desktop && npm test                      # 桌面全部回归测试
AGENT_DOCTOR_CHECK_DESKTOP=1 ./scripts/check.sh desktop   # 或在 macOS 上 ./scripts/check.sh all
```

涉及 Rust 子进程、Windows 路径、安装器：等 CI 的 `rust-windows` 绿了再合。发版走 `./scripts/check.sh release-preflight`。
