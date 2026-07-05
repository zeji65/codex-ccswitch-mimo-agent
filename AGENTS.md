# AGENTS.md

本文件适用于整个仓库。面向后续 AI/agent 协作，优先级高于一般 README，但低于用户当轮明确指令。

## Agent Kernel

- 始终使用简体中文，回答简洁清晰。
- 开始执行前先做 CAL，固定输出：`Goal / Constraints / Acceptance Criteria / Ambiguities / Skill`。
- CAL 完成前不得写文件。
- 只有歧义会影响结果、范围或风险判断时，`Ambiguities` 才非空并 HALT；轻微不确定可以记录假设后继续。
- `Skill` 声明本轮使用的 Skill/orchestrator；没有就写“无额外 Skill”。
- 不得编造路径、API、schema、配置、版本、仓库状态；不确定先读取真实文件或执行验证。
- 默认最小必要上下文；满足验收标准后立即收敛。

## Risk Gate

风险分级：`Standard` / `Strict`。

遇到 `Strict` 时，除非用户当轮已经明确授权，否则先 HALT 确认再执行。

`Strict` 范围：

- DB/schema、迁移、生产数据、备份/恢复策略。
- auth/security/secret、OAuth、token、账号池。
- infra/CI/deploy、发布、生产环境。
- 删除、清理、覆盖、批量文件操作。
- 公共 API、跨模块契约、Tauri command 入参/出参。
- 能力、规则、提示词系统，包括本文件和其它 agent 规则。

## WCH

对任何任务，先在内部确认目标、约束和验收标准，不把“已执行”当成“已完成”。

简单任务轻量完成；复杂、高风险、长链路或需要验收的任务，先验证再交付。

交付时优先说明结果如何回答用户目标，其次才说明文件、字段、脚本、测试或证据是否完整。

发现偏差时，先修根因，再按需修正产物、计划、脚本、知识或规则，避免把一次性问题固化成规则债务。

用户纠偏是判断校准的主要信号；具体工具如状态卡、门禁、回写阶梯只在任务需要时按需使用。

## Project Shape

CC Switch 是 Tauri 2 桌面应用：

- 前端：`src/`，React + TypeScript + Vite。
- 后端：`src-tauri/src/`，Rust + Tauri command。
- 数据库：`src-tauri/src/database/`，SQLite + rusqlite。
- 测试：`tests/` 前端/Hook 测试，Rust 测试在 `src-tauri/src/**` 或 `src-tauri/tests/`。
- 用户文档：`README.md`、`README_ZH.md`、`README_JA.md`、`docs/user-manual/`。
- 内部维护入口：`docs/PROJECT-README.md`。
- 可复跑 SOP：`docs/WORKFLOW-SOP.md`。

## Worktree Rules

- 仓库可能已有用户或其它 agent 的未提交改动；不得回滚、覆盖或格式化无关文件。
- 搜索优先使用 `rg`/`rg --files`。
- 手工编辑文件使用 `apply_patch`。
- 不使用破坏性命令，例如 `git reset --hard`、`git checkout -- <file>`，除非用户明确要求。
- 不把历史文档改写成当前事实；需要时添加当前状态说明或链接。

## Validation

按变更风险选择最小充分验证：

- TypeScript：`pnpm typecheck`。
- 前端单测：`pnpm vitest run <test-file>`，较大改动可跑 `pnpm test:unit`。
- Rust 单测：`cargo test <filter> --manifest-path src-tauri/Cargo.toml`，较大改动可跑 `cargo test --manifest-path src-tauri/Cargo.toml`。
- UI 变更：启动本地开发服务后，用浏览器实际查看关键页面和交互状态。
- 文档变更：至少检查目标文件存在、关键路径真实、没有明显过期版本或 TODO 占位。

最终回复要说明：改了什么、验证了什么、是否还有未验证风险。不要声称未执行的测试已经通过。

## Database Rules

- 当前 schema 版本以 `src-tauri/src/database/mod.rs` 中的 `SCHEMA_VERSION` 为准；写本文时源码为 `10`，未来必须重新读取确认。
- 表结构和迁移逻辑在 `src-tauri/src/database/schema.rs`。
- 备份、导入、导出逻辑在 `src-tauri/src/database/backup.rs`。
- 生产用户数据库通常位于 `~/.cc-switch/cc-switch.db`；测试不得直接写生产数据。
- 任何 schema 变更必须先说明迁移路径、备份/回滚策略、兼容边界，并补目标测试。
- 不要用旧版本打包 App 去验证新版本数据库；版本混用会造成兼容报错，不等于数据库损坏。

## Auth And Account Pool Rules

- 不输出 access token、refresh token、OAuth 原始 JSON、session secret。
- Codex OAuth 账号池逻辑集中在 `src-tauri/src/proxy/providers/codex_oauth_auth.rs`。
- Tauri auth command 集中在 `src-tauri/src/commands/auth.rs`，前端 API 在 `src/lib/api/auth.ts`。
- 账号导入、切换、token 获取必须通过现有 manager/command 路径，不直接手改运行中的账号池文件。
- 导入重复账号不得静默覆盖；除非用户明确要求覆盖或合并。

## Delivery Checklist

- 先回答用户目标是否完成，再列文件和测试。
- 明确区分本轮改动与仓库中原本存在的脏改。
- 涉及长期维护时，检查 `docs/PROJECT-README.md` 或 `docs/WORKFLOW-SOP.md` 是否需要同步。
