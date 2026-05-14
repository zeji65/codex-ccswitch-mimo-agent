# CC Switch Provider 重构交接文档

更新时间：2026-05-03 10:51:10 CST  
项目路径：`/Users/huzeji/cc-switch`  
当前分支：`codex/token`

## 1. 这份文档的目的

这不是一份“修一个具体 bug”的交接，而是一份“当前路线已经走偏，需要重构”的交接。

用户已经明确表示：

1. 不要再继续在现有问题堆里做零碎定位和补丁。
2. 不要继续把时间花在“到底是哪一坨逻辑又坏了”的排查上。
3. 需要回到官方可行路线，再在其基础上做一次二开重构。
4. 希望新会话能够尽量自主推进，并一次性把主线跑通。

换句话说，这份文档的目标不是让下一个会话继续补锅，而是让它：

1. 理解当前问题为什么不能再按旧方式修。
2. 理解用户真正想要的产品体验是什么。
3. 理解当前仓库里哪些改动只是探索性产物，不能默认当成最终方案。
4. 在官方方案和当前仓库之间重新做一次技术路线收口。

## 2. 用户真正想要的最终体验

用户理想中的使用方式非常简单：

1. 打开 `CC Switch`
2. 在 `Codex` 这一栏看到几种 provider
   - `OpenAI Official`
   - `DeepSeek`
   - `MiMo`
3. 想用哪个就切哪个
4. 回到 `Codex Desktop` 继续正常用
5. 不需要手动修改客户端
6. 不需要手动跑额外脚本
7. 不需要理解协议转换、路由重写、会话注入这些内部实现

用户期望的体感：

- 想稳一点、原生一点：切 `OpenAI Official`
- 想省成本、跑国内模型：切 `DeepSeek`
- 想测试小米链路：切 `MiMo`

背后的理想机制应该是：

1. `Codex Desktop` 继续发它当前那套请求
2. `CC Switch` 在中间做代理/兼容层
3. 如果当前 provider 是 `DeepSeek` 或 `MiMo`，就把请求转成它们能吃的格式
4. 再把结果转回 `Codex Desktop`

但用户不希望自己感知到这些技术细节。对用户来说，应该只是：

**“我切 provider，剩下都让 cc-switch 处理。”**

## 3. 用户为什么认为当前路线已经废掉了

这是整个交接里最关键的背景。

用户已经明确反馈：当前路线的问题不是单点 bug，而是技术路线本身已经偏了。

用户的核心判断是：

1. 之前尝试“在当前官方账号里切到另一个模型”的这条线，本质上就有问题。
2. 即使现在某些情况下“看起来切成功了”，也常常只是切到了一个 API 模式的新会话。
3. 这个新会话既不能保留本地原有会话，也不能真正连续下去。
4. 所以继续在这条线上补协议、补桥接、补切换、补账号池，只会越来越复杂。

用户还补充了两个非常重要的事实判断：

1. **当前“切模型”的实现常常实际上是新的 API 会话，而不是原生会话的继续。**
2. **本地路由监听端口/接管方式，可能已经和官方当前推荐或默认方案不一致。**

因此用户不希望新会话继续做这些事情：

1. 不要继续在现有实现上反复定位哪个切换点出了问题。
2. 不要继续假设“只要把 Responses -> DeepSeek/MiMo 转换补好就行”。
3. 不要继续把“切 provider”和“保留原生会话”混成同一个问题。
4. 不要继续把当前这套账号池 / live config / 本地路由叠逻辑的方式当成最终方向。

用户给出的新要求是：

**先读官方文档/官方已走通方案，再重构。**

## 4. 用户对新会话的授权方式

用户已经明确授权：

1. 可以自主规划
2. 不需要频繁来回确认
3. 任务量大也可以一次性推进
4. 重点是把主线跑通，而不是做零碎小修

但这不等于可以盲改。更准确地说：

- 可以少确认
- 但要先读官方方案，再设计重构主线

## 5. 当前这条路线里已经做过的事情

下面这些改动和尝试都已经发生过。它们里有些可以复用，有些只是探索性产物，不能默认继承为最终架构。

### 5.1 更新隔离 / 二开封包相关

已经做过的事：

1. 关闭了自定义构建的更新检测
2. 关闭了 updater artifact 生成
3. About 页面相关的更新入口被隐藏/禁用
4. 当前二开包不会再直接指向上游更新链路

相关文件：

- `src/lib/updater.ts`
- `src/contexts/UpdateContext.tsx`
- `src/components/settings/AboutSection.tsx`
- `src-tauri/tauri.conf.json`

这一部分总体方向是对的，应该大概率保留。

### 5.2 Codex OAuth / 账号池相关

已经做过的事：

1. 给账号加过 `auth.json` 快照/持久化
2. 做过“当前官方账号”的导入与切换
3. 做过“官方账号切换尽量不影响其他 provider”的收口
4. 做过一些 UI/文案调整，比如把“会话已过期”改成更接近“登录态失效”的表述
5. 做过一些减少自动刷新/减少状态抖动的改动

相关文件包括：

- `src-tauri/src/commands/auth.rs`
- `src-tauri/src/services/codex_desktop.rs`
- `src/components/providers/forms/hooks/useManagedAuth.ts`

这一部分最大的问题不是“不能工作”，而是它与 provider 切换、本地路由、live config 注入耦合太深。是否保留，要看新架构如何划分“认证层”和“模型/provider 层”。

### 5.3 Codex -> 第三方 provider 兼容链路

已经做过的事：

1. 为 `Codex` 加过一版 `apiFormat` 判断
2. 为 `Codex` 加过一版 `Responses -> Anthropic` 的桥接
3. 为 `MiMo/DeepSeek` 做过 anthropic bridge 的认证头适配
4. 试图让 `Codex Desktop` 的 `Responses` 请求，通过 `CC Switch` 转给第三方 provider

核心相关文件：

- `src-tauri/src/proxy/providers/codex.rs`
- `src-tauri/src/proxy/forwarder.rs`
- `src-tauri/src/proxy/handlers.rs`
- `src-tauri/src/proxy/providers/transform_responses.rs`
- `src-tauri/src/proxy/providers/streaming_responses.rs`
- `src-tauri/src/services/stream_check.rs`
- `src/config/codexProviderPresets.ts`
- `src/components/providers/forms/ProviderForm.tsx`

用户当前明确认为：这条线不能再按“补桥接细节”的方式往下走，而要先重新确认“官方现在允许/推荐的介入方式到底是什么”。

### 5.4 provider 切换 / 路由接管 / live config 恢复

已经做过的事：

1. 改过 `Codex official provider` 的应用路径
2. 做过“切模型/provider 与账号切换解耦”的尝试
3. 做过 runtime-only apply 和 restart path 的折中
4. 做过“从第三方切回官方时尽量恢复官方快照”的逻辑

相关文件：

- `src-tauri/src/services/provider/mod.rs`
- `src-tauri/src/services/codex_desktop.rs`

这一部分是当前问题最集中的地方之一，因为：

- 切换
- 接管
- 恢复
- 启停代理
- 保留 live config

这些动作现在相互影响太深。

## 6. 当前工作区的实际文件状态

当前分支：

- `codex/token`

当前工作区存在未提交修改，`git status --short` 显示的文件有：

- `CHANGELOG.md`
- `src-tauri/src/commands/auth.rs`
- `src-tauri/src/proxy/forwarder.rs`
- `src-tauri/src/proxy/handlers.rs`
- `src-tauri/src/proxy/providers/codex.rs`
- `src-tauri/src/proxy/providers/mod.rs`
- `src-tauri/src/proxy/providers/streaming_responses.rs`
- `src-tauri/src/proxy/providers/transform_responses.rs`
- `src-tauri/src/services/codex_desktop.rs`
- `src-tauri/src/services/provider/mod.rs`
- `src-tauri/src/services/stream_check.rs`
- `src-tauri/tauri.conf.json`
- `src/components/providers/forms/ProviderForm.tsx`
- `src/components/providers/forms/hooks/useManagedAuth.ts`
- `src/components/settings/AboutSection.tsx`
- `src/config/codexProviderPresets.ts`
- `src/contexts/UpdateContext.tsx`
- `src/lib/updater.ts`

新会话在接手时，不要默认这些改动全部正确，也不要默认都应该保留。

## 7. 当前已验证的本地真实状态

这一节只记录我已经通过本地命令验证过的事实。

### 7.1 当前 `CC Switch` 设置状态

来自 `~/.cc-switch/settings.json`：

- `enableLocalProxy = true`
- `visibleApps.codex = true`
- 当前 `Codex` provider：
  - `currentProviderCodex = "e772e1a6-4c69-4df7-800f-60f77b9450de"`

这个 `e772...` 当前对应的是：

- `Xiaomi MiMo`
- `category = cn_official`
- `apiFormat = anthropic`

也就是说：**从 settings / DB 视角看，当前 provider 已经是 MiMo。**

### 7.2 当前 providers 表中 Codex 记录

数据库 `~/.cc-switch/cc-switch.db` 中，`providers` 表里 `codex` 相关项已验证如下：

- `e772e1a6-4c69-4df7-800f-60f77b9450de` → `Xiaomi MiMo` → `cn_official` → `is_current = 1` → `api_format = anthropic`
- 另外有 3 条 `OpenAI Official`

也就是说：**从数据库 is_current 视角看，当前 provider 也已经是 MiMo。**

### 7.3 当前 `~/.codex/config.toml` 的 live 配置

最近一次读取结果显示：

```toml
model_provider = "xiaomi_mimo"
model = "mimo-v2-pro"
disable_response_storage = true
model_reasoning_effort = "xhigh"

[model_providers.xiaomi_mimo]
name = "xiaomi_mimo"
base_url = "https://api.xiaomimimo.com/anthropic"
wire_api = "responses"
requires_openai_auth = true
model_context_window = 1000000
model_auto_compact_token_limit = 9000000
```

这说明：**某个时刻 live config 也确实被切成过 MiMo。**

### 7.4 当前 `proxy_config` 状态

数据库 `proxy_config` 表中，`codex` 当前状态被验证为：

- `proxy_enabled = 0`
- `enabled = 0`
- `live_takeover_active = 0`
- `listen_address = 127.0.0.1`
- `listen_port = 15721`

这意味着：

**当前时刻本地代理接管并没有保持开启状态。**

### 7.5 当前 `auth.json`

最近一次读取到的 `~/.codex/auth.json` 中有：

- `auth_mode = "chatgpt"`
- `OPENAI_API_KEY = "PROXY_MANAGED"`

说明曾有一段时间代理接管过 live auth。

## 8. 当前已验证的关键现象

### 8.1 不开路由时的错误

用户复现到的错误：

```text
unexpected status 404 Not Found
url: https://api.xiaomimimo.com/anthropic/responses
```

这个现象说明：

1. 请求没有经过本地代理兼容层
2. Codex 当前直接按 `responses` 去打第三方 upstream
3. MiMo upstream 并不接受这个路径

所以这个 404 不应被理解成“MiMo provider 坏了”，而更像：

**“当前链路是 direct 模式，没经过兼容层。”**

### 8.2 开路由时出现过的错误

用户复现到的错误包括：

```text
unexpected status 502 Bad Gateway
url: http://127.0.0.1:15721/v1/responses
```

这说明有一段时间请求确实进入了本地代理。

### 8.3 MiMo 切换与代理接管短暂成功，但很快恢复

日志里验证到一个很关键的时间线：

```text
10:43:51 代理服务器启动于 127.0.0.1:15721
10:43:51 Codex Live 配置已接管
10:44:07 代理接管模式：热切换 codex 的目标供应商为 e772... (Xiaomi MiMo)
10:45:07 Codex Live 配置已恢复
10:45:07 代理服务器已停止
```

这说明一件非常重要的事：

**代理接管和 MiMo 热切换不是完全没成功，而是成功后没有稳定保持住。**

因此，当前问题不只是“MiMo 不兼容”，还包括：

1. 接管保持不住
2. 代理会在之后被恢复/停止
3. 用户此时再去 Codex 里发请求，就可能已经又回到 direct 模式

### 8.4 最新会话里“我是 Codex”的回答不能证明它仍然在走 MiMo

用户看到的回答像：

```text
我是 Codex，一个可以帮你写代码、查文件和解决问题的 AI 协作者。
```

这不能作为“MiMo 没生效”的充分证据，因为：

1. 这可能是旧运行态
2. 这可能是已经恢复到官方/原链路后的结果
3. 这也可能是当前会话上下文继续影响结果

因此，“回答像 Codex”不等于“当前请求一定走的是官方”或“一定没走 MiMo”。

## 9. 当前路线为什么被判定为架构性问题，而不是单点 bug

用户当前已经不认可继续补 bug 的原因主要有四个：

### 9.1 会话连续性问题

用户的核心诉求之一是：

- 切 provider 之后，最好仍然能像继续使用原生 Codex 一样
- 至少不要让它完全变成一个新的、与本地历史割裂的 API 会话

但当前路线里，“切模型/provider”与“live config 注入/API 形态变化”绑得太深，很容易变成：

1. 切到第三方
2. 实际上变成新的 API 会话
3. 本地历史/当前线程连续性体验被破坏

### 9.2 direct 模式与 proxy takeover 模式边界不清

当前实现里，同时存在：

1. provider 直接写入 live config
2. 本地代理 takeover
3. 热切换 provider
4. auth/token 同步
5. 恢复 live config
6. 停止代理服务

这些动作之间的边界现在不够清晰，结果是：

- 有时 live config 已经像 MiMo
- 但接管已经恢复
- 有时 settings/current provider 是 MiMo
- 但请求却没有走 MiMo

### 9.3 “切 provider”和“保留原生会话”被混在了一起

用户已经明确指出：

> 目前这样切，能切，但它是用 API 去登录的，所以会话既没有保留我本地的会话，它是新的会话，而且又不能够真正地继续下去。

这句话非常关键，说明现在的问题不是“再补一个 endpoint rewrite 就完了”，而是：

**第三方 provider 接入方式本身，正在影响会话模型。**

### 9.4 用户判断当前本地路由骨架可能都不对

用户明确提到：

> 我本地路由监听的端口应该跟你的端口是不一样的。

这意味着新会话不能再默认：

1. 当前 `15721`
2. 当前 takeover 流程
3. 当前 provider 注入方式
4. 当前 direct/proxy 切换方式

就是官方推荐或合理的骨架。

## 10. 当前最应该被推翻的旧假设

新会话不要再默认以下假设成立：

### 10.1 不要再默认“只要把 Responses -> MiMo/DeepSeek 转换补完，就能得到正确体验”

这可能只解决一部分协议兼容，不解决：

- 会话连续性
- 当前运行态
- 路由接管保持
- 官方链路与第三方链路的边界

### 10.2 不要再默认“把第三方 provider 写进 `~/.codex/config.toml` 就是正确主线”

目前还没有重新对齐官方方案前，这件事本身就应该被重新评估。

### 10.3 不要再默认“当前账号池 + provider 切换 + 本地路由”这三条线的组合方式是对的

它们现在明显已经耦合过深。

### 10.4 不要再默认“修复 direct 模式和 proxy 模式切换 bug”就是最终答案

用户已经不希望继续围绕这套交错状态机补锅。

## 11. 当前最应该保留的高层目标

虽然实现路线要重构，但下面这些高层目标仍然成立：

1. `OpenAI Official` 必须保留为稳定原生链路
2. `DeepSeek` 应该作为低成本日常主力
3. `MiMo` 应该作为补充测试链路
4. 用户最好只理解“切 provider”，不要理解实现细节
5. 不要要求用户手动跑第二个转换器/脚本
6. 尽量保留原生会话体验，至少不要明显退化

## 12. 新会话最推荐的工作方式

### 第一阶段：重新对齐官方可行路线

在动任何重构代码前，先核对：

1. 官方当前对 `Codex Desktop` / provider / routing / local proxy 的推荐或真实工作方式
2. 当前官方是否本来就有某种 routing/provider 注入机制
3. 官方当前 live config / 认证 / 会话是怎么组织的
4. 官方现有方案里，第三方接入应落在哪一层

这里的关键词是：

**不要先改仓库，要先校准“官方骨架”。**

### 第二阶段：重新划分边界

建议至少把下面几层重新拆开：

1. **认证层**
   - 官方账号
   - token / auth.json
   - 登录态恢复

2. **模型/provider 层**
   - 当前选中 provider
   - 第三方模型映射
   - 官方 provider 与第三方 provider 的表示

3. **接管/路由层**
   - 是否启用本地代理
   - 如何保持接管
   - 什么情况下恢复 live config

4. **会话体验层**
   - 是否能保留原生线程
   - 哪些情况下只能接受新线程
   - 用户层如何感知这种差异

### 第三阶段：明确一个新的接入主线

新会话应该重新回答下面这几个问题：

1. 第三方 provider 是否还应该写进 `Codex` live config
2. 第三方 provider 更适合通过：
   - 原生 provider 注入
   - 本地代理接管
   - 或官方支持的其他机制
3. “切 provider”时到底要不要动当前 `Codex` 原生认证态
4. 如何在第三方链路下尽量不破坏本地会话体验

### 第四阶段：再决定哪些旧改动保留

建议把现有改动分成三类：

1. **大概率保留**
   - updater 关闭
   - 基础 provider 预设
   - 一些通用 UI/配置能力

2. **待重新评估**
   - Codex OAuth 与 provider 切换的耦合方式
   - MiMo/DeepSeek bridge 的接入层
   - provider 切换后的 runtime 行为

3. **大概率应推倒重做**
   - 当前“切 provider + takeover + 恢复 + 停服”之间的交错状态管理
   - 以现有补丁逻辑继续扩展的方案

## 13. 对新会话最重要的提醒

### 13.1 用户现在最讨厌的不是 bug 本身，而是“越修越复杂”

所以不要继续做这种事情：

1. 再补一个 if
2. 再加一个 fallback
3. 再加一个恢复分支
4. 再给 MiMo 特判一下
5. 再补一个 direct / proxy 特殊路径

用户已经明确认为这条路径会继续把系统带进“屎堆里找哪坨屎有问题”。

### 13.2 用户不是要“证明 MiMo 能打通一次”，而是要一个可长期使用的产品体验

这意味着重构验收标准不应该只是：

- 某次请求成功了

而应该至少包含：

1. provider 切换路径清晰
2. 官方链路稳定
3. 第三方链路能用
4. 接管/恢复边界可理解
5. 不明显破坏会话体验

### 13.3 “MiMo/DeepSeek 一键切换”是产品目标，不是已经被当前实现证明的技术事实

当前实现并没有真正稳定证明这一点，所以新会话不要把它当成“只差最后一刀”的问题。

## 14. 当前最值得新会话立刻验证的问题

如果新会话要开始工作，最值得先回答的不是某个 404/502，而是：

1. 官方当前推荐的接入骨架到底是什么
2. 当前把第三方 provider 写进 Codex live config 这件事本身是否合理
3. 本地代理为什么会短暂接管后又恢复
4. 当前 provider 切换为什么会与会话体验发生强耦合
5. 要保原生会话体验的话，第三方 provider 应该落在哪一层

## 15. 当前仓库里可以直接用来复查的本地事实

新会话如果要复查现场，以下路径很重要：

### 项目

- `/Users/huzeji/cc-switch`

### 当前 app 配置与数据库

- `~/.cc-switch/settings.json`
- `~/.cc-switch/cc-switch.db`
- `~/.cc-switch/logs/cc-switch.log`

### 当前 Codex live 文件

- `~/.codex/config.toml`
- `~/.codex/auth.json`

### 可以帮助理解当前改动的核心代码

- `src-tauri/src/services/provider/mod.rs`
- `src-tauri/src/services/codex_desktop.rs`
- `src-tauri/src/services/proxy.rs`
- `src-tauri/src/proxy/forwarder.rs`
- `src-tauri/src/proxy/handlers.rs`
- `src-tauri/src/proxy/providers/codex.rs`
- `src-tauri/src/proxy/providers/transform_responses.rs`
- `src-tauri/src/proxy/providers/streaming_responses.rs`
- `src/config/codexProviderPresets.ts`
- `src/components/providers/forms/ProviderForm.tsx`
- `src/components/settings/ProxyTabContent.tsx`
- `src/components/proxy/ProxyToggle.tsx`

## 16. 一句话总结给新会话

这次接手的不是一个“把 MiMo 404/502 修掉”的问题，而是一个：

**“当前二开 provider/路由/会话方案已经被用户判定走偏，需要回到官方骨架重新做一版更清晰的接入架构。”**

如果只追求“某次请求打通”，很可能还会继续掉回原来的坑里。

