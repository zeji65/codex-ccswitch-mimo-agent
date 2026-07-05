# CC Switch Project README

这是项目内部维护入口，面向维护者和后续 AI 协作。根目录 `README.md` 是公开产品介绍，不在这里重复。

## Where Things Live

```text
.
├── AGENTS.md                 # AI/agent 执行规则和风险门禁
├── README.md                 # 面向用户的公开产品 README
├── README_ZH.md              # 中文公开产品 README
├── README_JA.md              # 日文公开产品 README
├── docs/
│   ├── PROJECT-README.md     # 当前内部维护入口
│   ├── WORKFLOW-SOP.md       # 可复跑协作流程
│   ├── user-manual/          # 多语言用户手册
│   ├── release-notes/        # 历史发布说明
│   └── superpowers/plans/    # 历史或专题开发计划
├── src/                      # React/TypeScript 前端
├── src-tauri/src/            # Rust/Tauri 后端
├── tests/                    # 前端、Hook、组件测试
└── package.json              # 前端脚本入口
```

如以后出现批量输入、可复跑分析或人工校正稿，优先放在任务专属 `runs/YYYY-MM-DD/` 下；原始输入放 `raw/`，人工口径放 `overrides.md`，运行记录放 `run-log.md`，最终产物放 `output.md`。

## Quick Commands

- 本地开发：`pnpm dev`
- 构建：`pnpm build`
- TypeScript 检查：`pnpm typecheck`
- 前端单测：`pnpm vitest run <test-file>`
- 全量前端单测：`pnpm test:unit`
- Rust 定向测试：`cargo test <filter> --manifest-path src-tauri/Cargo.toml`
- Rust 全量测试：`cargo test --manifest-path src-tauri/Cargo.toml`

## Current Architecture Anchors

- 应用版本：以 `package.json` 和 `src-tauri/Cargo.toml` 为准；写本文时两者都是 `3.16.5`。
- 数据库 schema：以 `src-tauri/src/database/mod.rs` 的 `SCHEMA_VERSION` 为准；写本文时源码为 `11`。
- 数据库迁移：`src-tauri/src/database/schema.rs`。
- 数据库备份/导入/导出：`src-tauri/src/database/backup.rs`。
- Codex OAuth 账号池：`src-tauri/src/proxy/providers/codex_oauth_auth.rs`。
- Tauri auth command：`src-tauri/src/commands/auth.rs`。
- Codex OAuth quota / models command：`src-tauri/src/commands/codex_oauth.rs`。
- Codex OAuth models service：`src-tauri/src/services/codex_oauth_models.rs`。
- 前端 auth API：`src/lib/api/auth.ts`。
- Codex OAuth UI：`src/components/providers/forms/CodexOAuthSection.tsx`。
- provider 表单状态：`src/components/providers/forms/hooks/useManagedAuth.ts`。

## Common Workflows

### UI Feature

1. 读现有组件、hook、测试，确认设计系统和状态流。
2. 先补目标组件或 hook 测试。
3. 小范围实现，不重排无关 UI。
4. 跑 `pnpm typecheck` 和定向 `pnpm vitest run <test-file>`。
5. 交互明显变化时启动 `pnpm dev`，用浏览器验证关键页面。

### Rust/Tauri Feature

1. 从 command、manager、service 的真实边界读起。
2. 先补 Rust 单测或前后端契约测试。
3. 保持 Tauri command 入参/出参向后兼容；若必须改契约，按 Strict 处理。
4. 跑定向 `cargo test <filter> --manifest-path src-tauri/Cargo.toml`。

### Database Work

1. 先读 `SCHEMA_VERSION`、`schema.rs`、相关 DAO 和历史迁移。
2. 明确生产数据风险、备份策略、forward-only 迁移路径。
3. 不直接写 `~/.cc-switch/cc-switch.db` 做实验；使用临时目录或测试数据库。
4. 版本混用要单独排查：旧 App 打开新 DB 的报错不等同于 DB 损坏。

### Auth / Account Pool Work

1. 先确认是否涉及 token、OAuth JSON、账号池文件或默认账号切换；涉及即 Strict。
2. 不在日志、测试快照、最终回复中暴露 token 或原始认证 JSON。
3. 账号导入走 Rust manager 和 Tauri command，不绕过状态管理直接改文件。
4. 验证重复账号、显示名、默认账号、token 获取路径不会影响其它账号。
5. 外部 account-pool JSON 可能多个邮箱共用同一个上游 `account_id`；源码需要区分 CC Switch 管理槽位 ID 和上游真实 `account_id`，转发注入的 `ChatGPT-Account-Id` 必须使用上游 ID。

## State Maintenance

做完会改变未来操作方式的任务后，检查这些状态面是否需要同步：

- `AGENTS.md`：规则、风险边界、必须 HALT 的情况。
- `docs/PROJECT-README.md`：入口、目录、命令、关键代码锚点。
- `docs/WORKFLOW-SOP.md`：可复跑步骤、验收检查、人工介入点。
- `docs/user-manual/` 或 release notes：只有用户可见行为稳定后再更新。

历史计划和发布说明保留历史语境；不要为了“看起来最新”改写旧记录。
