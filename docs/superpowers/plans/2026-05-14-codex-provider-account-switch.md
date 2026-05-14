# Codex Provider Account Switch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use inline execution in this session. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make selecting a Codex provider in CC Switch write the real Codex Desktop runtime auth/config and force Codex Desktop to reload the selected provider/account.

**Architecture:** Keep the latest upstream provider model and add back the proven runtime switch boundary from the local stable fix. Codex provider switching should use a dedicated `codex_desktop` service that backs up runtime files, writes the selected provider, syncs MCP, and restarts Codex Desktop on macOS with rollback on failure.

**Tech Stack:** Rust/Tauri backend, React/TypeScript frontend types for managed auth status, existing SQLite provider store, existing `~/.codex/auth.json` and `~/.codex/config.toml` writers.

---

## Task State

目标态：最新版分支中，用户点击任一 Codex provider 的“启用/使用”后，`~/.codex/auth.json` 与 `~/.codex/config.toml` 对齐目标 provider，Codex Desktop 被安全重启或提示无法重启，避免继续使用旧官方账号。

当前状态：`codex/upstream-latest-2026-05-14` 是最新上游干净分支；旧可用修复提交是 `d821a109 Fix Codex OAuth account switching`；旧封包主力环境不启动、不改动。

关键决策：不整坨 cherry-pick；只移植账号切换必须的 runtime 服务与命令入口；Codex 进程识别使用更窄规则，避免杀掉 `Contents/Resources/codex app-server` 这类宿主进程。

质量门禁：
- `ProviderService::switch(... AppType::Codex ...)` 在正常模式下通过 Codex runtime switch 服务写入真实 live 配置。
- Codex runtime switch 有备份、回滚、MCP 同步和 macOS 重启策略。
- Codex OAuth 管理器能保存/导入原生 `auth.json` 快照，用于真正切到 ChatGPT 账号。
- 不打印、不写入真实密钥；测试只使用假 token。
- 聚焦测试通过：`codex_desktop`、`codex_oauth_auth`、`commands::auth`、`provider` 相关测试或 `cargo check`。

已知风险：无法在本轮启动 GUI 验证 Codex Desktop 是否实际刷新；验收以代码路径、文件写入测试和进程匹配测试为准。

下一步：按任务逐项执行，并在失败时自主迭代到可交付状态。

---

## Files

- Create: `src-tauri/src/services/codex_desktop.rs`
  - Owns Codex runtime file backup/restore, account id parsing, safe macOS process stop/start, and provider switch transaction.
- Modify: `src-tauri/src/services/mod.rs`
  - Expose `codex_desktop` service to backend modules.
- Modify: `src-tauri/src/services/provider/mod.rs`
  - Route normal Codex provider switching through `codex_desktop::switch_desktop_to_provider`.
- Modify: `src-tauri/src/proxy/providers/codex_oauth_auth.rs`
  - Persist native Codex `auth.json` snapshots and expose `build_native_auth`, `import_current_native_auth`, `sync_current_native_auth_if_tracked`.
- Modify: `src-tauri/src/commands/auth.rs`
  - Add current Codex account status fields and commands for importing/switching current Codex account.
- Modify: `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`
  - Register new commands and service module.
- Modify: `src/lib/api/auth.ts`
  - Add frontend API types/methods for current account and direct switch commands.
- Optional Modify: `src/components/providers/forms/hooks/useManagedAuth.ts`, `src/components/providers/forms/CodexOAuthSection.tsx`
  - Surface direct account switch/import only if backend migration requires frontend path for existing provider forms.

---

## Task 1: Runtime Switch Service

**Files:**
- Create: `src-tauri/src/services/codex_desktop.rs`
- Modify: `src-tauri/src/services/mod.rs`

- [ ] Add `CodexRuntimeBackup` with `auth_bytes` and `config_bytes`.
- [ ] Add `read_current_codex_auth_account_id()` that reads only `tokens.account_id` from `~/.codex/auth.json`.
- [ ] Add `backup_runtime_files()` and `restore_runtime_files()` using existing atomic write helpers.
- [ ] Add `switch_desktop_to_provider()`:
  - Resolve provider live parts.
  - Validate target auth has `tokens.account_id` when it is a native ChatGPT auth provider.
  - Stop Codex Desktop on macOS.
  - Backup runtime files.
  - Optionally persist provider/current marker.
  - Call `write_live_with_common_config`.
  - Call `McpService::sync_all_enabled`.
  - Relaunch Codex Desktop.
  - Roll back provider/current marker/runtime files if any write fails.
- [ ] Add `switch_desktop_to_provider_with_runtime_auth_sync()` for account-pool switching without killing the current host before write.
- [ ] Add tests for:
  - `tokens.account_id` extraction.
  - Missing tokens returns `None`.
  - macOS process matcher accepts Codex main/helper and rejects unrelated apps plus `Contents/Resources/codex app-server`.

## Task 2: Route Codex Provider Switch Through Runtime Service

**Files:**
- Modify: `src-tauri/src/services/provider/mod.rs`

- [ ] In `ProviderService::switch_normal`, keep existing behavior for non-Codex apps.
- [ ] For `AppType::Codex`, after backfill and before returning, call `codex_desktop::switch_desktop_to_provider(state, provider, None, true)`.
- [ ] Avoid double writing live config/MCP for Codex by returning after the Codex runtime service succeeds.
- [ ] Preserve existing warning behavior for backfill failures.
- [ ] Add/adjust tests so Codex switch writes expected auth/config in a temp HOME and updates both local settings and database current provider.

## Task 3: Native Codex OAuth Snapshot Support

**Files:**
- Modify: `src-tauri/src/proxy/providers/codex_oauth_auth.rs`

- [ ] Extend stored account data with optional `id_token` and `native_auth`.
- [ ] Build native auth JSON using `auth_mode = "chatgpt"`, `OPENAI_API_KEY = null`, `tokens.access_token`, `tokens.refresh_token`, `tokens.account_id`, optional `tokens.id_token`, and `last_refresh`.
- [ ] On device login and token refresh, persist native auth snapshots when token material is available.
- [ ] Implement `import_current_native_auth()` from current `~/.codex/auth.json`.
- [ ] Implement `sync_current_native_auth_if_tracked()` that updates only already managed accounts.
- [ ] Implement `build_native_auth(preferred_account_id)` that returns the stored native snapshot and fails clearly if snapshot is missing.
- [ ] Add tests for native auth shape, import current auth, sync tracked auth, read-only token from persisted snapshot, and missing snapshot errors.

## Task 4: Auth Commands And Frontend API

**Files:**
- Modify: `src-tauri/src/commands/auth.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/lib/api/auth.ts`

- [ ] Add `current_account_id` and `current_account_login` to managed auth status.
- [ ] Add `auth_import_current` command for Codex OAuth.
- [ ] Add `auth_switch_current_account` command for Codex OAuth using the runtime switch service.
- [ ] Register both commands in Tauri invoke handler.
- [ ] Add frontend API methods `authImportCurrent` and `authSwitchCurrentAccount`.
- [ ] Add backend tests for rejecting direct Codex account switch when current provider is not official and for provider auth binding.

## Task 5: Verification

**Commands:**
- [ ] `cargo test codex_desktop --lib`
- [ ] `cargo test codex_oauth_auth --lib`
- [ ] `cargo test commands::auth --lib`
- [ ] `cargo test provider::tests --lib`
- [ ] `cargo check`
- [ ] `git diff --stat`
- [ ] Secret scan over changed files for real-looking `tp-...` or long `sk-...` keys.

Expected pass condition：all targeted tests and `cargo check` pass, or any failures are unrelated pre-existing failures explicitly documented with evidence.

---

## Execution Result

Status: PASS

Implemented:
- Codex normal provider switch now goes through `codex_desktop::switch_desktop_to_provider`, which writes real `~/.codex/auth.json` / `~/.codex/config.toml`, syncs MCP, and restarts Codex Desktop on macOS.
- Codex hot-switch under proxy takeover now restarts Codex Desktop after provider selection.
- Codex hot-switch re-applies local proxy takeover when the database has a live backup but the actual live Codex config has drifted back to direct official/API config.
- Codex OAuth account pool now persists native `auth.json` snapshots and exposes import/switch commands for direct Codex account switching.
- Frontend managed-auth API/types now include current Codex account status and import/switch methods.

Verified:
- `cargo test codex_desktop --lib`
- `cargo test codex_oauth_auth --lib`
- `cargo test commands::auth --lib`
- `cargo test switch_codex_provider_writes_live_config_and_current_markers --lib`
- `cargo test hot_switch_codex_reapplies_takeover_when_backup_exists_but_live_is_direct --lib`
- `cargo check`
- `pnpm typecheck`
- Changed-file secret scan for long `tp-...` / `sk-...` patterns: no match.
