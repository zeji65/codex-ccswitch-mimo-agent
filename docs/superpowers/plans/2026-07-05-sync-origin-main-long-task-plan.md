# Sync Origin Main Long Task Goal Plan

计划日期：2026-07-05

当前目标：在保留本地二次开发分支 `正式版V2` 的基础上，安全吸收 CC Switch 官方远端 `origin/main` 的大量更新，并让后续 AI 能从本计划冷启动继续执行、暂停和验收。

## Goal Contract

完成态：

- 本地二次开发成果被保护，不因同步官方远端而丢失。
- 官方远端 `origin/main` 的目标更新进入一个可验证的集成分支。
- 冲突被按风险分层解决：普通代码和文档可自主处理，auth、OAuth、账号池、Tauri command、DB、公共 API 相关决策必须 HALT。
- 核心二开能力仍可被证据验证，包括 Codex OAuth 账号池、导入当前 Codex 登录、账号切换、quota 展示、代理转发路径。
- 验证结果、剩余风险、是否合回 `正式版V2` 的建议被记录。

完成证据：

- `git status --short --branch` 显示集成分支无未处理冲突。
- 计划或执行日志记录保护方式、集成分支名、`origin/main` 哈希、合并结果。
- `pnpm typecheck` 通过。
- 相关前端测试或 Rust 测试按触达面通过，跳过项有明确理由。
- 最终报告列出本轮改动与原本脏改的边界。

## Scope / Non-goals

范围内：

- 读取仓库规则、当前 Git 状态、两侧提交差异和受影响文件。
- 保护当前工作区中的二次开发改动。
- 从 `origin/main` 建立官方远端更新清单。
- 在新集成分支里合并官方远端更新。
- 按风险解决冲突并运行最小充分验证。
- 产出最终集成报告和后续合回建议。

范围外：

- 未经用户明确授权，不直接改写 `正式版V2` 历史。
- 未经用户明确授权，不强推、发布、部署、删除大量文件、重置工作区。
- 不写生产数据库，不直接操作 `~/.cc-switch/cc-switch.db`。
- 不输出 token、OAuth 原始 JSON、session secret、账号池敏感内容。
- 不把历史文档改写成当前事实；需要时只追加当前状态说明或链接。

## Ambiguities / Assumptions

| ID | Type | Question / Unknown | Default / Decision | Owner | Resolve By | Effect On Plan |
|---|---|---|---|---|---|---|
| AMB-001 | assumed | CC Switch 的远端具体指哪个 remote | 按 `origin` 处理，地址为 `https://github.com/farion1231/cc-switch.git` | planner | planning | 任务围绕 `origin/main` 同步 |
| AMB-002 | assumed | 本地二次开发主线是哪条分支 | 按当前分支 `正式版V2` 处理 | planner | planning | 集成从 `正式版V2` 派生 |
| AMB-003 | deferred | 当前未提交改动用 WIP commit 还是 stash 保护 | 执行期先推荐 WIP commit，但必须让用户确认 | user | before A004 | 影响保护产物和 forbidden replay |
| AMB-004 | deferred | 集成完成后是否自动合回 `正式版V2` 并推送 | 默认只产出集成分支，合回和推送需用户确认 | user | before A015 side effect | 防止误改主线和远端 |
| AMB-005 | deferred | 官方远端最新状态是否仍是当前本地 tracking 看到的 75 commits behind | 执行第一轮必须 `git fetch origin` 后重算 | execution AI | A002-A005 | 防止用陈旧远端引用规划冲突 |
| AMB-006 | blocking | 用户是否授权执行本计划中的 Git side effects | 本轮只有规划授权；执行需用户后续明确说开始 | user | before execution | 未授权前不得执行任务池 |

## Decision Log

| ID | Date | Decision | Source | Applies To | Effect | Resume Note |
|---|---|---|---|---|---|---|
| DEC-001 | 2026-07-05 | 用户纠正“远端”指 CC Switch 官方远端，不是二开远端 `mimo-app` | user | Goal Contract | 同步来源固定为 `origin/main` | 从 Fact Source Map 和 A001 继续 |
| DEC-002 | 2026-07-05 | 用户要求使用 long-task-goal-planner 规划对应任务 | user | full plan | 本轮只生成和保存计划，不执行任务池 | 执行需后续明确授权 |
| DEC-003 | 2026-07-05 | 计划落点选择已有目录 `docs/superpowers/plans/` | verified file | plan artifact | 不新建顶层目录 | 保存本文件后运行计划校验 |
| DEC-004 | 2026-07-05 | 用户要求“按计划执行” | user | A001-A003 | 授权开始执行第一轮；保护方式仍需在 A004 前确认 | 执行到 AMB-003 后 HALT |
| DEC-005 | 2026-07-05 | 已执行 `git fetch origin`；`origin/main` 更新到 `7a7d41c873c1efe32d9caf4733f44c57d3a07fee` | command output | A002 | 当前分叉为 `ahead 6 / behind 384`，merge-base 为 `7685ab7049bbd447c155dd172ac67c769b33948c` | 继续 A003 保护清单 |
| DEC-006 | 2026-07-05 | A003 保护清单完成；本地 tracked 改动 14 个文件、untracked 8 个文件；明显长 key/token 模式扫描无匹配文件 | command output | A003 | 下一步必须选择 WIP commit、stash 或人工整理 | 从 A004 前的 AMB-003 HALT 继续 |
| DEC-007 | 2026-07-05 | 用户选择保护方式 1：WIP commit | user | A004 | 使用保护分支和 WIP commit 保护当前二开改动 | 已创建保护分支 `codex/protect-v2-wip-2026-07-05` |
| DEC-008 | 2026-07-05 | 已在保护分支创建 WIP 保护提交 `4edebc6592b9fed01a865ebe3007320ba2ac83b3` | command output | A004 | 当前二开改动已形成可恢复快照，`正式版V2` 指针未被推进 | 继续 A005 创建集成分支 |
| DEC-009 | 2026-07-05 | 已从受保护状态创建集成分支 `codex/sync-origin-main-2026-07-05` | command output | A005 | 集成工作与保护分支分离 | 继续 A006 merge |
| DEC-010 | 2026-07-05 | 已执行 `git merge --no-commit --no-ff origin/main`，merge 停在冲突状态，无 merge commit | command output | A006 | 官方更新已展开到集成分支；剩余 8 个冲突文件 | 继续 A007 风险地图 |
| DEC-011 | 2026-07-05 | A008 已解决 2 个前端 Standard 冲突：`ProviderCard.tsx` 和 `ProviderForm.tsx` | command output | A008 | 保留本地 Codex OAuth/managed account UI，同时接入上游 Codex Chat routing/API format/catalog/custom headers 字段 | A009 需处理后端 Strict/Mixed 冲突 |
| DEC-012 | 2026-07-05 | 用户要求“继续处理 A009”；A009 后端 Mixed/Strict 冲突已按手工融合策略解决 | user + command output | A009 | 保留本地 Codex Desktop 重启、Codex tail repair、Live re-takeover 行为，同时接入上游 ActiveConnectionGuard、Codex Chat transform、catalog/display 更新和 Claude placeholder cleanup | 继续 A010/A011 验证 |
| DEC-013 | 2026-07-05 | A010 前端验证通过：`pnpm typecheck`，以及 `ProviderForm.codexCatalog`、`ProviderCardLayout`、`useCodexConfigState.catalog` 定向测试 | command output | A010 | 前端类型和冲突相关 UI/Hook 测试通过 | 继续 A012 能力保留清单 |
| DEC-014 | 2026-07-05 | A011 Rust 验证通过：`cargo fmt --check`、`cargo check`、`cargo test hot_switch_codex`、`cargo test codex_tail_repair`、`cargo test switch_codex_provider_writes_live_config_and_current_markers` | command output | A011 | 后端冲突融合可格式化、可编译，Codex 热切换和 tail repair 定向回归通过 | 继续 A012 能力保留清单 |
| DEC-015 | 2026-07-05 | A012 核心二开能力保留清单完成；Codex OAuth 账号池、导入当前登录、账号切换、quota 展示、proxy forwarder 路径均有代码和测试证据 | command output + code inspection | A012 | 未发现本地二开能力被上游合并吞掉；未使用真实 token 或生产 DB | 继续 A013 审查和 secret scan |
| DEC-016 | 2026-07-05 | A013 审查与 changed diff secret scan 完成；未发现 P0/P1，secret scan 命中均为公开示例、文档锚点、测试/占位值 | command output + redacted inspection | A013 | 当前集成结果无阻断审查项；仍未做真实 GUI/真实账号手动验证 | 继续 A014 状态维护 |
| DEC-017 | 2026-07-05 | A014 更新 `docs/PROJECT-README.md`：版本/schema 从 `3.14.1`/`10` 修正为 `3.16.5`/`11`，并补 Codex OAuth quota/models 锚点 | source inspection + docs patch | A014 | 未来 agent 不会按旧版本/schema 或缺失锚点行动；`docs/WORKFLOW-SOP.md` 无需改动 | 继续 A015 最终交付和合回决策 |

## A003 Protection Inventory

当前本地改动需要先保护，不能直接 merge `origin/main`。

| File | State | Category | Risk | Upstream Overlap | Handling Note |
|---|---|---|---|---|---|
| `src-tauri/src/commands/auth.rs` | modified | auth / Tauri command | Strict | no direct file overlap | 保护后合并；后续若改 command 契约需 HALT |
| `src-tauri/src/commands/import_export.rs` | modified | import/export / account data | Mixed | no direct file overlap | 保护后合并；涉及账号导入导出语义需复核 |
| `src-tauri/src/lib.rs` | modified | Tauri command registration | Strict | yes | 高冲突风险；后续合并需确认命令注册不丢 |
| `src-tauri/src/proxy/forwarder.rs` | modified | proxy forwarding | Mixed | yes | 直接重叠；后续需要看官方 proxy 更新和本地 header/account 行为 |
| `src-tauri/src/proxy/providers/codex_oauth_auth.rs` | modified | Codex OAuth account pool | Strict | yes | 最高风险；不得机械采用任一侧 |
| `src-tauri/tauri.conf.json` | modified | Tauri config | Mixed | yes | 版本、bundle 或配置冲突需人工确认 |
| `src/components/CodexOauthQuotaFooter.tsx` | modified | quota UI | Standard | no direct file overlap | 保护后合并；后续跑组件测试 |
| `src/components/providers/ProviderCard.tsx` | modified | provider UI | Standard | yes | 直接重叠；后续检查官方 UI 改动 |
| `src/components/providers/forms/CodexOAuthSection.tsx` | modified | Codex OAuth UI | Mixed | no direct file overlap | 保护后合并；涉及导入当前登录交互 |
| `src/components/providers/forms/hooks/useManagedAuth.ts` | modified | managed auth state | Mixed | no direct file overlap | 保护后合并；涉及 auth 状态契约需复核 |
| `src/lib/api/auth.ts` | modified | frontend auth API | Strict | no direct file overlap | 保护后合并；Tauri invoke contract 变更需 HALT |
| `src/lib/api/settings.ts` | modified | frontend settings API | Mixed | yes | 直接重叠；需保留官方 settings 更新和本地字段 |
| `src/lib/query/subscription.ts` | modified | subscription query / quota | Standard | yes | 直接重叠；后续关注官方 free-plan quota 更新 |
| `vite.config.ts` | modified | test/build config | Standard | no direct file overlap | 保护后合并；跑 typecheck/tests |
| `AGENTS.md` | untracked | agent rules | Strict | no direct file overlap | 规则文件；是否纳入保护提交需用户明确接受 |
| `docs/PROJECT-README.md` | untracked | internal maintenance docs | Standard | no direct file overlap | 可纳入 WIP 保护，后续如路径变化再更新 |
| `docs/WORKFLOW-SOP.md` | untracked | workflow SOP | Standard | no direct file overlap | 可纳入 WIP 保护，后续如流程变化再更新 |
| `docs/superpowers/plans/2026-07-05-sync-origin-main-long-task-plan.md` | untracked | execution plan | Standard | no direct file overlap | 当前进度源，建议纳入保护 |
| `tests/components/CodexOAuthSection.importJson.test.tsx` | untracked | frontend test | Standard | no direct file overlap | 可纳入 WIP 保护 |
| `tests/components/CodexOauthQuotaFooter.test.tsx` | untracked | frontend test | Standard | no direct file overlap | 可纳入 WIP 保护 |
| `tests/components/ProviderCard.codexQuota.test.tsx` | untracked | frontend test | Standard | no direct file overlap | 可纳入 WIP 保护 |
| `tests/hooks/useCodexOauthQuotaRefresh.test.ts` | untracked | hook test | Standard | no direct file overlap | 可纳入 WIP 保护 |

A003 evidence:

- `git fetch origin` succeeded on 2026-07-05.
- `origin/main`: `7a7d41c873c1efe32d9caf4733f44c57d3a07fee`.
- Divergence after fetch: `HEAD...origin/main = 6 / 384`.
- Merge base: `7685ab7049bbd447c155dd172ac67c769b33948c`.
- Direct tracked overlap with upstream: `src-tauri/src/lib.rs`, `src-tauri/src/proxy/forwarder.rs`, `src-tauri/src/proxy/providers/codex_oauth_auth.rs`, `src-tauri/tauri.conf.json`, `src/components/providers/ProviderCard.tsx`, `src/lib/api/settings.ts`, `src/lib/query/subscription.ts`.
- Direct untracked overlap with upstream: none.
- Long `sk-...`, `tp-...`, JWT-like key scan over changed/untracked files: no matching file.

## A006 / A007 Conflict Risk Map

Merge command:

```bash
git merge --no-commit --no-ff origin/main
```

Merge result: automatic merge failed; no merge commit was created.

| File | Conflict Count | Category | Risk | Current Status | Required Decision |
|---|---|---|---|---|---|
| `src/components/providers/ProviderCard.tsx` | 1 | provider UI / quota / routing badge | Standard | resolved and staged | Keep both local Codex OAuth quota logic and upstream Codex routing badge logic |
| `src/components/providers/forms/ProviderForm.tsx` | 1 | provider form UI | Standard | resolved and staged | Keep local `CodexOAuthSection` and upstream Codex `apiFormat`, reasoning, catalog, user-agent, request override fields |
| `src-tauri/src/provider.rs` | 1 | provider model / usage result | Mixed | resolved and staged | Kept local `UsageResult::not_supported` helpers while preserving upstream ProviderTestConfig wording |
| `src-tauri/src/services/mod.rs` | 1 | service module exports | Mixed | resolved and staged | Included both local `codex_desktop` and upstream `codex_oauth_models` exports |
| `src-tauri/src/services/provider/mod.rs` | 2 | provider switching / tests / live config | Strict | resolved and staged | Preserved local Codex Desktop restart on switch and upstream provider/common-config tests/validation behavior |
| `src-tauri/src/proxy/handlers.rs` | 2 | proxy request/response handling | Strict | resolved and staged | Preserved local Codex tail repair path while using upstream Codex Chat-to-Responses transform where routing requires it |
| `src-tauri/src/proxy/response_processor.rs` | 1 | response processor / SSE logging | Strict | resolved and staged | Fused local Codex tail repair stream with upstream `ActiveConnectionGuard` lifetime handling |
| `src-tauri/src/services/proxy.rs` | 8 | proxy service / live takeover / backup/restore | Strict | resolved and staged | Preserved local Codex Live re-takeover behavior while keeping upstream catalog/display update and Claude placeholder cleanup behavior |

A008 Standard resolution evidence:

- Conflict-marker search over `ProviderCard.tsx` and `ProviderForm.tsx` produced no matches.
- `git diff --check -- src/components/providers/ProviderCard.tsx src/components/providers/forms/ProviderForm.tsx` passed.
- Both resolved frontend files were staged with `git add`.

A009 backend resolution evidence:

- Conflict-marker search over all 8 conflict files produced no matches.
- `git diff --check` over the 6 backend conflict files passed.
- `git diff --name-only --diff-filter=U` returned no files.
- `src-tauri/src/proxy/handlers.rs` keeps the upstream Codex Chat-to-Responses transform for routed Codex Chat providers, and uses local Codex tail repair for native Codex Responses passthrough.
- `src-tauri/src/proxy/response_processor.rs` now passes `ActiveConnectionGuard` through streaming, non-streaming, and Codex tail repair paths.
- `src-tauri/src/services/proxy.rs` keeps Codex Live takeover reapplication when backup exists but Live drifted direct, while retaining upstream provider display/catalog updates and Claude placeholder cleanup.

A010 / A011 validation evidence:

- `pnpm typecheck` passed.
- `pnpm vitest run tests/components/ProviderForm.codexCatalog.test.ts tests/components/ProviderCardLayout.test.ts tests/hooks/useCodexConfigState.catalog.test.ts` passed: 3 files, 5 tests.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` passed.
- `cargo check --manifest-path src-tauri/Cargo.toml` passed.
- `cargo test hot_switch_codex --manifest-path src-tauri/Cargo.toml` passed: 3 tests.
- `cargo test codex_tail_repair --manifest-path src-tauri/Cargo.toml` passed: 2 tests.
- `cargo test switch_codex_provider_writes_live_config_and_current_markers --manifest-path src-tauri/Cargo.toml` passed: 1 test.

A012 capability preservation checklist:

| Capability | Evidence | Validation | Residual Risk |
|---|---|---|---|
| Codex OAuth 账号池 | `src-tauri/src/proxy/providers/codex_oauth_auth.rs` still owns `CodexOAuthManager`, account store, default account, external JSON import, duplicate protection, native auth snapshot, token refresh, upstream account id mapping | `cargo test codex_oauth --manifest-path src-tauri/Cargo.toml` passed 40 tests | 未使用真实账号登录流；网络 OAuth 端到端需人工验证 |
| 导入当前 Codex 登录 | UI `CodexOAuthSection` calls `useManagedAuth.importCurrentAccount`; frontend API invokes `auth_import_current`; Rust command calls `import_current_native_auth` | `pnpm vitest run tests/components/CodexOAuthSection.importJson.test.tsx ...` passed; `cargo test codex_oauth` includes native snapshot parsing tests | 未读取真实 `~/.codex/auth.json`，避免暴露 token |
| 账号切换 / 默认账号 | `useManagedAuth` still exposes `setDefaultAccount` and `switchCurrentAccount`; Rust `auth_switch_current_account` calls Codex Desktop switch path and records `authBinding` | `cargo test managed_account --manifest-path src-tauri/Cargo.toml` passed 9 tests; `cargo test set_codex_account_binding --manifest-path src-tauri/Cargo.toml` passed | 未实际重启/切换真实 Codex Desktop |
| quota 展示 | `ProviderCard.tsx` detects `meta.authBinding.source=managed_account` + `authProvider=codex_oauth`; `CodexOauthQuotaFooter` uses `useCodexOauthQuota` and account id resolution | Codex OAuth quota frontend tests passed 4 files / 7 tests | quota API 网络返回未用真实账号验证 |
| proxy forwarder | `src-tauri/src/proxy/forwarder.rs` still resolves managed Codex OAuth account, fetches token from `CodexOAuthManager`, injects upstream `chatgpt-account-id`, session cache headers, and rejects proxy placeholder upstream | `cargo test codex_oauth` includes session header and placeholder guard tests; `cargo test managed_account` includes takeover placeholder tests | 未跑真实上游请求，避免使用真实 token |

A013 review and secret scan evidence:

- Conflict fusion review checked `process_response`, `process_response_with_codex_tail_repair`, Codex Chat transform call sites, `ActiveConnectionGuard` propagation, Codex hot-switch/takeover paths, and Claude placeholder cleanup call sites.
- `git diff --cached --name-only --diff-filter=U` returned no files.
- `git diff --cached --check` passed.
- Staged conflict-marker scan over changed text files produced no matches.
- Changed-diff secret scan initially flagged potential patterns; redacted inspection classified them as false positives:
  - `sk-` pattern matched markdown anchor text such as `disk-...`.
  - AWS key pattern matched public S3 example/test credentials.
  - sensitive assignment pattern matched docs, i18n placeholders, and tests using fake values.
- Review finding: no P0/P1 blocking findings found. Residual risk: no real GUI session, real Codex OAuth account, or real upstream proxy request was exercised.

A014 state maintenance evidence:

- State surfaces checked: `docs/PROJECT-README.md`, `docs/WORKFLOW-SOP.md`, package manifests, database schema version source.
- `docs/PROJECT-README.md` updated because its version/schema anchors were stale after the upstream sync: `3.14.1` → `3.16.5`, `SCHEMA_VERSION 10` → `11`.
- `docs/PROJECT-README.md` also now lists Codex OAuth quota/models command and service anchors.
- `docs/WORKFLOW-SOP.md` required no change: workflow steps and validation thresholds remain accurate.

## Autonomy Model

自主等级：semi-autonomous after approval。

AI may decide:

- 如何读取最小必要事实源。
- 如何把冲突按 docs、frontend、Rust、auth、proxy、DB、config 分类。
- 如何解决明显机械冲突和非 Strict 文档冲突。
- 如何选择最小充分验证命令。
- 如何记录执行结果和恢复锚点。

AI may not decide:

- 是否强推、重写历史、删除大量文件、重置工作区。
- 是否直接合回 `正式版V2` 或推送远端。
- 是否改变 auth、OAuth、账号池、Tauri command、DB schema、公共 API 的行为契约。
- 是否泄露或复制真实 token、OAuth 原始 JSON、session secret。

User retains:

- 开始执行本计划的授权。
- 当前未提交改动的保护方式选择。
- Strict 冲突的业务取舍。
- 集成分支是否合回 `正式版V2` 和是否推送。

Strict actions:

- auth、security、secret、OAuth、token、账号池相关变更。
- Tauri command 入参/出参、公共 API、跨模块契约变更。
- DB schema、迁移、备份、生产数据。
- infra、CI、deploy、发布。
- 删除、批量覆盖、强推、历史重写。
- AGENTS、Skill、提示词、规则系统变更。

Allowed writes after execution approval:

- 新建保护分支或 WIP commit。
- 新建集成分支。
- 修改冲突文件、测试、必要文档。
- 更新本计划的 Decision Log 和 Resume Anchor。

Forbidden writes without explicit approval:

- `git reset --hard`。
- `git checkout -- <file>` 用于丢弃他人或用户改动。
- `git push --force`。
- 写入生产 DB 或账号池运行文件。
- 删除大量文件或清理未知生成物。

Allowed external effects:

- `git fetch origin` 更新本地远端引用。
- 在用户批准后推送集成分支到指定远端。

Forbidden external effects:

- 未经确认发布版本、创建 release、部署、推送主线、强推。

## Fact Source Map

| Type | Path / Source | Purpose | Read Status | Confidence |
|---|---|---|---|---|
| user input | current conversation on 2026-07-05 | 目标、远端纠正、规划请求 | read | high |
| repo rules | `/Users/huzeji/cc-switch/AGENTS.md` | CAL、Risk Gate、验证和工作区规则 | read | high |
| project index | `/Users/huzeji/cc-switch/docs/PROJECT-README.md` | 目录、命令、架构锚点 | read | high |
| workflow SOP | `/Users/huzeji/cc-switch/docs/WORKFLOW-SOP.md` | 可复跑流程和产物边界 | read | high |
| existing plan | `/Users/huzeji/cc-switch/docs/superpowers/plans/2026-05-14-codex-provider-account-switch.md` | 历史二开计划风格和核心能力线索 | read | medium |
| package scripts | `/Users/huzeji/cc-switch/package.json` | `pnpm typecheck`、`pnpm test:unit`、版本 | read | high |
| Rust manifest | `/Users/huzeji/cc-switch/src-tauri/Cargo.toml` | Rust package version and cargo entry | read | high |
| git status | `git status --short --branch` on 2026-07-05 | 当前分支和未提交改动 | read | high |
| git remotes | `git remote -v` on 2026-07-05 | `origin` and `mimo-app` 地址 | read | high |
| branch tracking | `git branch -vv` on 2026-07-05 | `正式版V2` tracks `mimo-app/正式版V2` | read | high |
| divergence count | `git rev-list --left-right --count HEAD...origin/main` on 2026-07-05 | 当前本地引用显示 ahead 6 behind 75 | read | medium |
| merge base | `git merge-base HEAD origin/main` on 2026-07-05 | 当前共同基线为 `7685ab7049bbd447c155dd172ac67c769b33948c` | read | medium |
| changed files | `git diff --name-only` and `git ls-files --others --exclude-standard` on 2026-07-05 | 本地二开触达面 | read | high |
| upstream commits | `git log --oneline HEAD..origin/main` on 2026-07-05 | 官方远端更新摘要 | read | medium |
| local commits | `git log --oneline origin/main..HEAD` on 2026-07-05 | 本地二开提交摘要 | read | high |

## Task Type Diagnosis

类型：mixed。

- development：需要在集成分支解决真实代码冲突并验证功能。
- refactor-migration：官方远端可能改动 proxy、provider、Codex Chat conversion、presets、docs 和格式化结构，合并时要保护兼容边界。
- audit-review：需要审查二开能力是否被官方更新覆盖。
- workflow-governance：本任务本身需要可恢复计划、Decision Log、HALT 和质量门禁。

任务切分原则：

- 每轮 3 个任务，降低跨模块合并的风险。
- Git side effects 和 Strict 冲突拆开，避免自动执行高风险取舍。
- `writing-plans` 只在具体开发单元冻结后使用。
- `code-reviewer` 只在集成结果可审查后使用。

## Task Pool

| ID | Goal | Why Now | Input Sources | Action | Output Artifact | Acceptance | Quality Gate | Risk | Dependencies | Downstream Skill | Downstream Fallback | HALT If | Resume Anchor | Autonomous |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| A001 | 重新装载规则和当前仓库事实 | 防止旧聊天状态误导执行 | user input; AGENTS.md; docs/PROJECT-README.md; docs/WORKFLOW-SOP.md; this plan | 读取规则、计划、状态命令，确认仍在目标仓库和目标分支 | 本计划 Decision Log 追加执行启动记录 | 记录当前日期、分支、规则版本和执行授权来源 | Gate R1: source reload checklist pass | Standard | none | none | not needed | 用户未明确授权执行任务池 | Resume Anchor section, current_round A01 | partial |
| A002 | 刷新官方远端更新事实 | 当前 `origin/main` 引用可能过期 | git remote -v; git branch -vv; origin/main | 执行 `git fetch origin`，重算 `HEAD...origin/main`，记录新哈希和 ahead behind | 本计划 Decision Log 记录 fetched origin hash and divergence | `origin/main` hash、merge base、ahead behind 均有记录 | Gate R1: refreshed git facts recorded | Standard | A001 | none | not needed | fetch 失败或 remote 指向不是 CC Switch 官方仓库 | Decision Log DEC next row | partial |
| A003 | 生成本地二开保护清单并请求保护方式 | 工作区当前有多处未提交改动 | git status; git diff --name-only; git ls-files --others --exclude-standard | 分类当前 tracked and untracked 改动，推荐 WIP commit 或 stash，等待用户选择 | 保护清单记录在本计划 Resume Anchor 或执行报告 | 每个本地改动文件都归类到 auth、proxy、UI、config、test、docs | Gate R1: protection inventory complete | Standard | A001 | none | not needed | 用户不选择保护方式或文件归属无法判断 | Resume Anchor open_gaps AMB-003 | no |
| A004 | 执行用户选择的保护动作 | 合并前必须避免脏工作区丢改 | A003 protection inventory; user decision | 按用户选择创建 WIP commit、保护分支或 stash，并记录引用 | Git commit hash、branch name or stash ref recorded | `git status --short` 与保护方式一致，未提交改动不再无保护 | Gate R2: protected worktree evidence exists | Standard | A003 and user decision | none | not needed | 保护动作会覆盖文件、包含敏感内容、或用户未确认 | Decision Log protection record | partial |
| A005 | 创建官方更新集成分支 | 避免直接污染 `正式版V2` | A002 refreshed origin hash; A004 protection artifact | 从受保护状态新建 `codex/sync-origin-main-2026-07-05` 或用户指定分支 | 新集成分支名和起点哈希记录 | `git status --short --branch` 显示位于集成分支 | Gate R2: integration branch confirmed | Standard | A004 | none | not needed | 分支名已存在且含未知工作，或切换会覆盖改动 | Resume Anchor current_round A02 | partial |
| A006 | 将 `origin/main` 合入集成分支 | 核心同步动作必须隔离执行 | integration branch; origin/main refreshed hash | 执行 merge，保留冲突状态，不做高风险自动取舍 | Merge result recorded; conflict list if any | merge 成功或冲突文件清单完整 | Gate R2: merge state classified | Mixed | A005 | none | not needed | merge 需要删除大量文件、触及 Strict 契约且无用户授权、或出现不可恢复 Git 状态 | Resume Anchor side_effects_done merge attempted | partial |
| A007 | 建立冲突和触达面风险地图 | 冲突解决前要先知道风险边界 | merge conflict list; AGENTS Risk Gate; changed files | 按 docs、frontend、Rust、auth、proxy、DB、config、tests 分类冲突和非冲突关键文件 | Conflict risk map appended to this plan or execution report | 每个冲突文件都有风险等级、owner area、建议处理路径 | Gate R3: conflict map complete | Standard | A006 | none | not needed | 冲突文件包含敏感数据或无法确定来源 | Resume Anchor artifacts_changed conflict map | partial |
| A008 | 解决 Standard 冲突 | 先收敛低风险冲突，减少噪音 | A007 conflict map; relevant source files | 处理 docs、format、普通 UI、测试导入等非 Strict 冲突 | Resolved Standard conflict diffs in integration branch | Standard 冲突不再出现在 `git status` unmerged list | Gate R3: no unresolved Standard conflicts | Standard | A007 | writing-plans | inline execution with stated limits | 需要重构跨模块契约或无法判断用户意图 | Resume Anchor last_completed_task_ids through A008 | partial |
| A009 | 处理 Strict 冲突的设计和授权 | auth、proxy、Tauri command、DB 不能机械合并 | A007 conflict map; AGENTS Auth And Account Pool Rules; source anchors | 对 Strict 文件形成最小设计选项，HALT 给用户确认后再改 | Strict conflict decision note and approved resolution path | 每个 Strict 冲突都有保留本地、采用上游、手工融合或延期的决策 | Gate R3: Strict decision log complete | Strict | A007 | writing-plans | HALT | 用户未批准 Strict 取舍、涉及 token、DB schema、公共 API 或 Tauri command 契约变化 | Decision Log Strict decision rows | no |
| A010 | 运行前端类型和定向测试 | 前端和 API 层是当前二开触达面 | package.json; tests directory; changed UI and API files | 跑 `pnpm typecheck` 和相关 `pnpm vitest run` 测试，记录失败根因 | Validation result entries in execution report | 命令通过，或失败被归因并生成修复任务 | Gate R4: frontend validation evidence | Standard | A008 and A009 resolution if needed | none | not needed | 同一 required gate 连续失败两次或失败源于未授权契约变更 | Resume Anchor last_quality_gate frontend | partial |
| A011 | 运行 Rust 和 Tauri 定向验证 | auth、proxy、commands 是高风险二开核心 | Cargo.toml; src-tauri/src commands and proxy files | 跑相关 `cargo test` 或 `cargo check`，按触达面选择过滤器 | Rust validation result entries in execution report | 命令通过，或失败定位到具体模块和后续修复任务 | Gate R4: Rust validation evidence | Mixed | A008 and A009 resolution if needed | none | not needed | 测试需要生产 DB、真实 token、迁移授权或同一 gate 连续失败两次 | Resume Anchor last_quality_gate rust | partial |
| A012 | 验证核心二开行为未被吞掉 | 合并通过不等于功能保留 | historical plan; AGENTS Auth rules; relevant tests; UI files | 检查 Codex OAuth 账号池、导入当前登录、切换、quota、proxy forwarder 行为路径 | Capability preservation checklist in execution report | 每项能力有代码路径、测试、或明确未验证风险 | Gate R4: capability checklist complete | Mixed | A010 and A011 | code-reviewer | inline execution with stated limits | 发现能力被官方更新覆盖且修复涉及 Strict 契约 | Resume Anchor open_gaps capability checklist | partial |
| A013 | 做集成结果审查和 secret scan | 防止冲突修复引入隐蔽回归或泄密 | git diff; changed files; AGENTS secret rules | 审查 diff、运行 changed-file secret scan、确认无无关格式化 | Review findings and secret scan result in execution report | 无 P0/P1 未处理问题，无真实 token 或 key 模式命中 | Gate R5: review and secret scan pass | Mixed | A010-A012 | code-reviewer | inline execution with stated limits | 扫描命中疑似真实 secret 或审查发现 P0/P1 | Resume Anchor last_quality_gate review | partial |
| A014 | 判断项目状态文档是否需要同步 | 长期维护任务可能改变后续操作方式 | docs/PROJECT-README.md; docs/WORKFLOW-SOP.md; final diff | 只在路径、命令、核心锚点、SOP 发生实际变化时更新文档 | State maintenance note or small docs patch | 有更新则路径真实且无空泛条目；无更新则记录跳过理由 | Gate R5: state maintenance decision recorded | Standard | A013 | project-state-maintenance | inline execution with stated limits | 需要修改 AGENTS 或其它规则系统 | Resume Anchor artifacts_changed docs decision | partial |
| A015 | 形成最终交付和合回决策 | 集成分支完成后还不能默认改主线 | all validation results; git status; user decision if merging | 汇总结果，建议是否合回 `正式版V2`、是否推送、剩余风险 | Final integration report in final response and plan Decision Log | 报告包含完成情况、文件边界、验证结果、未验证风险、下一步选项 | Gate R5: final report complete | Mixed | A013-A014 | none | not needed | 用户要求合回、推送、发布、强推或删除时未明确授权 | Resume Anchor state complete or halt | no |

## Round Cadence

| Round | Task IDs | Goal | Quality Gates | Can Continue If | Resume Anchor |
|---|---|---|---|---|---|
| A01 | A001-A003 | 重载事实、刷新远端、明确保护方式 | Gate R1 source reload, refreshed git facts, protection inventory | 用户确认保护方式，或任务进入 HALT 等待确认 | Resume Anchor current_round A01, next A004 |
| A02 | A004-A006 | 保护本地二开并创建集成分支完成 merge 尝试 | Gate R2 protected worktree, branch confirmed, merge state classified | 工作区已保护，集成分支存在，merge 成功或冲突清单完整 | Resume Anchor current_round A02, next A007 |
| A03 | A007-A009 | 风险分层并解决冲突 | Gate R3 conflict map complete, Standard conflicts resolved, Strict decisions logged | 无未授权 Strict 冲突，或 HALT 已得到用户决策 | Resume Anchor current_round A03, next A010 |
| A04 | A010-A012 | 运行验证并确认核心二开能力保留 | Gate R4 frontend validation, Rust validation, capability checklist | required gates 通过，或失败已有根因和下一步任务 | Resume Anchor current_round A04, next A013 |
| A05 | A013-A015 | 审查、状态维护和最终交付 | Gate R5 review, secret scan, state maintenance, final report | 无 P0/P1、无 secret 命中、合回推送决策清楚 | Resume Anchor state complete or halt |

## Quality Gates

```yaml
quality_gate:
  round: A01
  task_ids: A001-A003
  task_type: mixed
  gates:
    - name: source reload checklist
      command_or_check: read AGENTS.md, project docs, this plan, git status, git remote
      expected: current rules and repository facts recorded before side effects
      actual: planned
      result: planned
      skip_reason: none
    - name: refreshed git facts
      command_or_check: git fetch origin, git rev-list, git merge-base
      expected: origin/main hash and divergence recorded after fetch
      actual: planned
      result: planned
      skip_reason: none
    - name: protection inventory
      command_or_check: git status, git diff --name-only, git ls-files --others --exclude-standard
      expected: all local tracked and untracked changes classified
      actual: planned
      result: planned
      skip_reason: none
  required_pass:
    - source reload checklist
    - refreshed git facts
    - protection inventory
  residual_risk: user must still choose WIP commit or stash
  can_continue: only after protection method is decided
  halt_reason: user decision missing for AMB-003
```

```yaml
quality_gate:
  round: A02
  task_ids: A004-A006
  task_type: migration
  gates:
    - name: protected worktree evidence
      command_or_check: git status and recorded commit, branch, or stash reference
      expected: no local secondary development change remains unprotected
      actual: planned
      result: planned
      skip_reason: none
    - name: integration branch confirmed
      command_or_check: git status --short --branch
      expected: current branch is integration branch, not 正式版V2
      actual: planned
      result: planned
      skip_reason: none
    - name: merge state classified
      command_or_check: git status unmerged list or clean merge result
      expected: merge success or complete conflict list
      actual: planned
      result: planned
      skip_reason: none
  required_pass:
    - protected worktree evidence
    - integration branch confirmed
    - merge state classified
  residual_risk: merge may create Strict conflicts
  can_continue: yes if conflicts are classified and no destructive action is needed
  halt_reason: destructive Git recovery or unapproved Strict conflict
```

```yaml
quality_gate:
  round: A03
  task_ids: A007-A009
  task_type: code
  gates:
    - name: conflict risk map
      command_or_check: inspect unmerged files and AGENTS Risk Gate
      expected: every conflict file has risk level and planned handling
      actual: 8 conflict files classified in A006 / A007 Conflict Risk Map
      result: pass
      skip_reason: none
    - name: Standard conflicts resolved
      command_or_check: git status unmerged list excludes Standard conflict files
      expected: Standard conflicts resolved without unrelated formatting
      actual: ProviderCard.tsx and ProviderForm.tsx resolved and staged
      result: pass
      skip_reason: none
    - name: Strict decisions logged
      command_or_check: Decision Log rows for every Strict conflict
      expected: no Strict conflict is resolved without user decision
      actual: DEC-012 records user continuation request and manual fusion strategy for all backend Mixed/Strict conflicts
      result: pass
      skip_reason: none
  required_pass:
    - conflict risk map
    - Standard conflicts resolved
    - Strict decisions logged
  residual_risk: user may choose to defer a Strict conflict
  can_continue: yes only after all Strict conflicts are approved or deferred with acceptance impact
  halt_reason: unapproved auth, DB, Tauri command, public API, or secret risk
```

```yaml
quality_gate:
  round: A04
  task_ids: A010-A012
  task_type: mixed
  gates:
    - name: frontend validation
      command_or_check: pnpm typecheck and targeted pnpm vitest run commands
      expected: pass or failure root cause recorded
      actual: pnpm typecheck passed; targeted vitest command passed 3 files / 5 tests
      result: pass
      skip_reason: none
    - name: Rust validation
      command_or_check: cargo test or cargo check with src-tauri/Cargo.toml
      expected: pass or failure root cause recorded
      actual: cargo fmt --check, cargo check, and 3 targeted cargo test filters passed
      result: pass
      skip_reason: none
    - name: capability preservation checklist
      command_or_check: inspect Codex OAuth, quota, provider card, auth API, proxy paths
      expected: every core二开能力 has evidence or residual risk
      actual: Codex OAuth account pool, import current login, account switching, quota display, and proxy forwarder paths have code and targeted test evidence
      result: pass
      skip_reason: none
  required_pass:
    - frontend validation
    - Rust validation
    - capability preservation checklist
  residual_risk: GUI behavior may still need manual app verification
  can_continue: yes if failures are fixed or explicitly accepted as unrelated
  halt_reason: same required gate fails twice or capability loss needs Strict decision
```

```yaml
quality_gate:
  round: A05
  task_ids: A013-A015
  task_type: audit
  gates:
    - name: review and secret scan
      command_or_check: git diff review plus changed-file secret scan
      expected: no P0 or P1 findings and no real-looking secret
      actual: no P0/P1 findings; changed-diff secret scan hits were redacted and classified as examples/placeholders/tests/markdown false positives
      result: pass
      skip_reason: none
    - name: state maintenance decision
      command_or_check: compare final diff with docs/PROJECT-README.md and docs/WORKFLOW-SOP.md
      expected: docs updated only if operation paths changed, otherwise skip reason recorded
      actual: docs/PROJECT-README.md updated for version/schema and Codex OAuth quota/models anchors; docs/WORKFLOW-SOP.md unchanged with skip reason
      result: pass
      skip_reason: none
    - name: final report
      command_or_check: final response and Decision Log
      expected: completion, verification, residual risk, and merge-back options stated
      actual: planned
      result: planned
      skip_reason: none
  required_pass:
    - review and secret scan
    - state maintenance decision
    - final report
  residual_risk: merge back and push remain user-owned decisions
  can_continue: complete if no HALT remains
  halt_reason: user requests merge back, push, release, force push, or destructive cleanup without explicit approval
```

Skipped gate policy:

- A skipped gate must record command or check, skip reason, residual risk, and why the round can or cannot continue.
- A required gate cannot be silently skipped.
- If the same required gate fails twice, HALT with root-cause hypothesis and recovery options.

Final gate:

- All task IDs A001-A015 are done, skipped with reason, or HALTed with a resumable decision.
- No unresolved conflict remains.
- No unapproved Strict side effect is hidden in the branch.
- Final report states whether the plan is complete, exhausted, or halted.

## HALT Conditions

Must HALT when:

- 用户尚未明确授权执行本计划任务池。
- 当前未提交改动的保护方式未确认。
- `origin` 不再指向 CC Switch 官方仓库，或 `origin/main` 不存在。
- Git 操作需要 reset hard、checkout discard、force push、history rewrite、delete大量文件。
- 合并冲突涉及 auth、OAuth、token、账号池、Tauri command、公共 API、DB schema、迁移、备份、生产数据。
- 冲突文件中出现真实 token、OAuth 原始 JSON、session secret 或疑似 secret。
- 同一 required quality gate 连续失败两次。
- 官方远端更新规模比当前估算大 5 倍以上，或冲突数量大到当前任务池无法覆盖。
- 需要发布、部署、创建 release、修改 CI 或 infra。
- 需要修改 AGENTS、Skill、提示词、hook、validator 或规则系统。

HALT output format:

```text
HALT reason:
ambiguity / risk type:
current state:
completed:
not completed:
side effects done:
side effects not done:
conflict / risk:
decision needed:
options:
recommendation:
resume from:
forbidden replay:
```

## Resume Anchor

```yaml
resume_anchor:
  current_round: A05 in progress
  next_task_ids:
    - A015
  last_completed_task_ids:
    - A001
    - A002
    - A003
    - A004
    - A005
    - A006
    - A007
    - A008
    - A009
    - A010
    - A011
    - A012
    - A013
    - A014
  required_sources_to_reload:
    - /Users/huzeji/cc-switch/AGENTS.md
    - /Users/huzeji/cc-switch/docs/PROJECT-README.md
    - /Users/huzeji/cc-switch/docs/WORKFLOW-SOP.md
    - /Users/huzeji/cc-switch/docs/superpowers/plans/2026-07-05-sync-origin-main-long-task-plan.md
    - git status --short --branch
    - git remote -v
    - git branch -vv
    - git rev-parse origin/main
    - git rev-list --left-right --count HEAD...origin/main
  artifacts_changed:
    - /Users/huzeji/cc-switch/docs/superpowers/plans/2026-07-05-sync-origin-main-long-task-plan.md
    - /Users/huzeji/cc-switch/docs/PROJECT-README.md
  decision_log_location: Decision Log section in this plan
  last_quality_gate: Gate R5 review/secret scan and state maintenance passed; final report/merge commit pending
  open_gaps:
    - AMB-004 merge back and push decision
    - A015 final report, integration merge commit, and merge-back recommendation
  side_effects_done:
    - created this plan file
    - fetched origin; origin/main is now 7a7d41c873c1efe32d9caf4733f44c57d3a07fee
    - updated this plan with A001-A003 evidence
    - created protection branch codex/protect-v2-wip-2026-07-05
    - created WIP protection commit 4edebc6592b9fed01a865ebe3007320ba2ac83b3
    - created progress commit 95bcc897cc4152cba3630d362cd81a9a668923df on protection branch
    - created integration branch codex/sync-origin-main-2026-07-05
    - attempted no-commit merge from origin/main; merge is currently in progress
    - resolved and staged frontend Standard conflicts in ProviderCard.tsx and ProviderForm.tsx
    - resolved and staged backend Mixed/Strict conflicts in provider.rs, services/mod.rs, services/provider/mod.rs, proxy/handlers.rs, proxy/response_processor.rs, and services/proxy.rs
    - ran frontend validation: pnpm typecheck and targeted vitest passed
    - ran Rust validation: cargo fmt --check, cargo check, and targeted cargo tests passed
    - completed A012 capability preservation checklist with targeted frontend/Rust tests
    - completed A013 review and changed-diff secret scan; matches classified as false positives/placeholders
    - updated docs/PROJECT-README.md for current version/schema and Codex OAuth quota/models anchors
  forbidden_replay:
    - do not create duplicate WIP commit or stash for A004
    - do not rerun merge if A006 already left a conflict state unless idempotency is checked
    - do not run `git merge --abort` unless user explicitly chooses to abandon this merge attempt
    - do not rerun secret-bearing inspections into logs
    - do not push, force push, reset, or delete files without explicit decision log approval
  safe_next_action: create the integration merge commit on codex/sync-origin-main-2026-07-05, then report merge-back/push options without performing them
```

## Downstream Skill Routing

| Situation | Downstream Skill | Handoff Input | Frozen Decisions | Expected Output | Quality Gate | Return To Long Task | Fallback |
|---|---|---|---|---|---|---|---|
| Standard conflict implementation unit becomes concrete | writing-plans | conflict map, target files, acceptance, tests | sync source is `origin/main`, base branch is integration branch | small implementation plan or direct scoped patch | target tests or typecheck pass | A008 or A010 | inline execution with stated limits |
| Strict auth、proxy、Tauri command conflict requires design | writing-plans | Strict conflict details, AGENTS Auth rules, relevant files | user must approve behavior contract before edit | concrete design and file-level plan | Decision Log approval plus targeted tests | A009 | HALT |
| Integration result needs acceptance review | code-reviewer | final diff, validation logs, capability checklist | review is findings-first, no unrelated refactor | P0/P1/P2 findings or no-issue statement | no unresolved P0/P1 | A013 | inline execution with stated limits |
| Project state surfaces may need update | project-state-maintenance | final diff and changed operation paths | do not rewrite history docs as current facts | docs update or skip reason | target docs exist and no placeholders | A014 | inline execution with stated limits |

Downstream fallback policy:

- If `writing-plans` is unavailable for Standard conflicts, execute inline only when the file set is small and acceptance is testable.
- If `writing-plans` is unavailable for Strict conflicts, HALT because safety ownership matters.
- If `code-reviewer` is unavailable, perform findings-first inline review with file and line references.
- If `project-state-maintenance` is unavailable, use AGENTS Delivery Checklist and docs/PROJECT-README.md State Maintenance section inline.

## Execution Handoff

```yaml
execution_handoff:
  cold_start_sources:
    - /Users/huzeji/cc-switch/docs/superpowers/plans/2026-07-05-sync-origin-main-long-task-plan.md
    - /Users/huzeji/cc-switch/AGENTS.md
    - /Users/huzeji/cc-switch/docs/PROJECT-README.md
    - /Users/huzeji/cc-switch/docs/WORKFLOW-SOP.md
    - git status --short --branch
    - git remote -v
    - git branch -vv
  current_progress_source: Resume Anchor and Decision Log sections in this plan
  how_to_pick_next_round: choose the first round whose task IDs are not completed, skipped with reason, or halted with unresolved decision
  decision_log_location: Decision Log section in this plan
  quality_gate_policy: required gates must pass or record failure root cause; same required gate failing twice HALTs
  downstream_fallback_policy: follow task-level fallback; if absent or risk changes, HALT
  plan_drift_policy: if user constraints or repo rules conflict with this plan, return to Goal Contract and update Decision Log before continuing
  halt_policy: use HALT output format and record side effects done plus forbidden replay
  forbidden_replay: do not repeat WIP commit, stash, merge, push, reset, secret scan logging, or docs rewrite without checking recorded side effects
  completion_report: final answer must state completion against user goal, files changed by this run, verification run, skipped gates, and residual risk
```

## First Round Recommendation

推荐第一轮只做 A001-A003，不直接进入 merge：

1. 重新读取 `AGENTS.md`、本计划、项目维护文档和当前 Git 状态。
2. 执行 `git fetch origin` 后刷新官方远端事实。
3. 生成本地二开改动保护清单，并让用户确认使用 WIP commit、stash，还是先人工整理。

推荐的用户选择：WIP commit 到临时保护分支。理由是二开改动规模较大，且包含 auth、proxy、UI、测试和文档，WIP commit 比 stash 更容易审计、恢复和接力。
