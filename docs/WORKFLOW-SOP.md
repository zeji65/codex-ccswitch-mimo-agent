# CC Switch Workflow SOP

本 SOP 用于把一次开发、调研、导入、验证流程变成后续 AI 可以复跑的工作方式。先定目录，再做步骤，最后才写原则。

## Directory Scheme

```text
.
├── AGENTS.md
├── docs/
│   ├── PROJECT-README.md
│   ├── WORKFLOW-SOP.md
│   └── superpowers/plans/
├── src/
├── src-tauri/
├── tests/
└── runs/
    └── YYYY-MM-DD-task-slug/
        ├── raw/           # 原始输入，永不直接编辑
        ├── overrides.md   # 用户确认的口径、命名、修正
        ├── run-log.md     # 实际执行命令和验证结果
        └── output.md      # 可交付结果或复盘摘要
```

`runs/` 只在任务产生可复用证据或多轮输入时创建；简单代码改动不需要硬建目录。

## SOP

| Step | Actor | Input | Action | Output Artifact | Location | Completion Check |
| --- | --- | --- | --- | --- | --- | --- |
| 1. Intake | AI | 用户需求、当前指令 | 输出 CAL，判断 Standard/Strict | CAL 记录 | 对话 | 目标、约束、验收标准清楚；Strict 已授权或已 HALT |
| 2. Discover | AI | 仓库真实文件 | 最小范围读取 README、AGENTS、package、相关源码/测试 | 事实清单 | 对话或 `runs/*/run-log.md` | 没有编造路径、版本、schema、命令 |
| 3. Place Artifacts | AI | 任务类型和已有目录 | 决定产物放哪里，避免双重归属 | 目录方案 | 文档或对话 | 原始输入、人工修正、脚本、输出各有唯一位置 |
| 4. Test First | AI | 预期行为 | 为 feature/bugfix 补定向测试；纯文档任务可跳过 | 测试文件或跳过理由 | `tests/` 或 `src-tauri/` | 测试能表达目标风险 |
| 5. Implement | AI | 测试和现有模式 | 小范围修改，保留无关脏改 | 代码/文档改动 | 原模块 | 不改无关文件，不泄露 secret |
| 6. Validate | AI | 改动范围 | 跑最小充分命令 | 验证结果 | 对话或 `run-log.md` | 命令通过，或失败原因清楚 |
| 7. Browser Check | AI | UI 改动 | 启动本地服务并实际查看关键交互 | 截图或观察记录 | 对话或 `run-log.md` | 页面可见、交互可用、布局无明显错位 |
| 8. Data Safety Check | AI | DB/auth/账号池相关改动 | 复核备份、迁移、token、默认账号影响 | 风险说明 | 对话或文档 | 不写生产 DB；不暴露 token；不影响其它账号 |
| 9. State Maintenance | AI | 已完成改动 | 判断是否更新 AGENTS、项目 README、SOP、用户手册 | 状态面更新或跳过理由 | 相关文档 | 未来 agent 不会按旧路径/旧命令行动 |
| 10. Deliver | AI | 验证结果 | 简短汇报完成情况、文件、测试、残余风险 | 最终回复 | 对话 | 回答用户目标，不把“已执行”误当“已完成” |

## Good Enough Thresholds

- 文档任务：文件存在、链接/路径真实、没有明显 TODO 占位，旧入口没有被误覆盖。
- UI 任务：定向测试 + typecheck 通过；关键交互浏览器验证通过。
- Rust/Tauri 任务：定向 Rust 测试通过；涉及前后端契约时前端调用层也验证。
- DB 任务：迁移测试、备份策略、版本边界清楚；禁止直接在生产 DB 上试错。
- Auth/账号池任务：token 不外泄；重复账号、默认账号、显示名、其它账号影响都被验证。

## AI Rules

- Artifact boundary is strict：上游原始输入不被直接编辑，下游输出不反写原始证据。
- Facts before inference：先读真实文件、schema、命令，再判断。
- Non-first runs force-read prior artifacts：继续任务时先读已有 `run-log.md`、`overrides.md`、相关测试和文档。
- User checkpoints stay meaningful：Strict 任务没有明确授权就 HALT。
- Keep indexes light：只有当累计产物变多时才新增索引，不为一次性任务制造重目录。
