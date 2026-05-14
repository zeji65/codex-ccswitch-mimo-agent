# Implementation Plan: Codex Provider Fix

## Overview

Restore packaged-version capabilities to the new CC Switch: add accounts (official + third-party), query quota/status (capability-aware), and switch Codex Desktop provider without crashing. Tasks are organized by **P0 / P1 / P2** priority, not by linear phases. Each task references the provider service chain (multiple files) rather than a single file.

## Guiding Principles

- **API Key is default, OAuth is explicit opt-in.** Auth path is decided by metadata (`meta.providerType == "codex_oauth"` or `meta.authBinding`), not by TOML fields like `requires_openai_auth`.
- **Auth construction is runtime-only.** Any auth object built during switch is written to `~/.codex/auth.json` only — never persisted back into the provider record, never logged with the raw key.
- **Quota query is capability-aware.** Official OAuth accounts must support quota; third-party providers report `unsupported` if no API exists. `unsupported` is not an error.
- **No file lock-in.** Tasks reference the provider service chain spanning `services/provider/{mod.rs, usage.rs}`, `services/codex_desktop.rs`, `commands/provider.rs`, and `src/hooks/useProviderActions.ts`.

## Provider Service Chain (referenced files)

- `src-tauri/src/services/codex_desktop.rs` — runtime switch service for Codex
- `src-tauri/src/services/provider/mod.rs` — switch routing and orchestration
- `src-tauri/src/services/provider/usage.rs` — usage script execution
- `src-tauri/src/commands/provider.rs` — Tauri IPC handlers (add/switch/usage)
- `src-tauri/src/proxy/providers/codex_oauth_auth.rs` — OAuth device code flow and snapshot
- `src/hooks/useProviderActions.ts` — frontend add/switch logic
- `src/lib/api/auth.ts` — frontend auth API
- Frontend usage display components (under `src/components/providers/`)

## Tasks

### P0 — Must Complete (release blockers)

- [x] 1. Fix switch crash with API-Key-first detection
  - [x] 1.1 Add `extract_api_key_from_settings()` helper
    - In `src-tauri/src/services/codex_desktop.rs`, add a private function that reads `settings.pointer("/auth/OPENAI_API_KEY")` and returns `Option<String>`
    - Function must NEVER log the extracted key value
    - _Requirements: 2.3_

  - [x] 1.2 Update `build_effective_codex_settings()` with API-Key-first rules
    - **Rule 1**: If provider is NOT explicitly OAuth (i.e., `meta.providerType != "codex_oauth"` AND no `codex_oauth` managed auth binding), skip OAuth snapshot generation entirely
    - **Rule 2**: For API Key providers without an `auth` field, construct `{"OPENAI_API_KEY": key}` from `extract_api_key_from_settings()` ONLY for the runtime write — do not mutate the stored provider record
    - **Rule 3**: Never reclassify an API Key provider as `codex_oauth`
    - **Rule 4**: Only providers explicitly opted into OAuth read the snapshot via `build_native_auth()`
    - _Requirements: 1.3, 1.4, 2.3, 6.1, 6.2, 6.3_

  - [x] 1.3 Update `validate_target_auth()` to accept API Key format
    - Accept `auth.OPENAI_API_KEY` (string) as valid → `Ok(())`
    - Keep existing OAuth validation: `auth_mode == "chatgpt"` requires `tokens.account_id`
    - Allow unknown auth shapes through (passthrough) instead of blanket rejection
    - _Requirements: 2.1, 2.2_

  - [x] 1.4 Verify switch failure rollback works for API Key path
    - In `services/codex_desktop.rs::switch_desktop_to_provider_inner()`, confirm rollback restores runtime files when `build_effective_codex_settings()` or `validate_target_auth()` returns Err
    - Confirm errors propagate as `AppError` to frontend, never as `panic!`
    - _Requirements: 3.1, 3.2, 3.3, 3.4_

- [x] 2. Add-account flow does not crash and shows in list
  - [x] 2.1 Verify API Key add path bypasses OAuth completely
    - Trace `commands/provider.rs::add_provider()` → DB save → `get_providers()` for a Codex provider with only `settings_config.auth.OPENAI_API_KEY`
    - Confirm no call into `CodexOAuthManager` is triggered
    - Confirm no `meta.authBinding` is auto-attached
    - _Requirements: 4.2, 4.3_

  - [x] 2.2 Verify OAuth add path failures return structured errors
    - In `proxy/providers/codex_oauth_auth.rs::poll_for_token()`, ensure all error variants map to `CodexOAuthError` with user-readable messages
    - Confirm device code expiration, network failure, and invalid state do not panic
    - _Requirements: 4.1, 4.4_

  - [x] 2.3 Verify frontend add-account path does not confuse OAuth and API Key
    - In `src/hooks/useProviderActions.ts::addProvider()`, ensure `authStartLogin("codex_oauth")` is invoked only when the user explicitly chose OAuth in the form
    - API Key providers go directly through `providersApi.add()`
    - _Requirements: 4.2, 4.3_

  - [x] 2.4 Verify added providers appear in list immediately
    - After `add_provider()` succeeds, confirm `get_providers("codex")` returns the new provider
    - Confirm React Query invalidation triggers list refresh in `useAddProviderMutation.onSuccess`
    - _Requirements: 4.2, 4.3_

- [x] 3. Restore official account quota query
  - [x] 3.1 Verify OAuth account token availability for usage queries
    - In the provider service chain (`commands/provider.rs::queryProviderUsage` → `services/provider/usage.rs`), confirm the OAuth access token can be retrieved via `CodexOAuthManager::get_valid_token()` for a given Codex OAuth provider
    - Confirm the token refresh path executes without crashing
    - _Requirements: 5.1_

  - [x] 3.2 Verify quota query uses correct endpoint for official accounts
    - Confirm the usage script template (or hardcoded path) targets the OpenAI/Codex usage endpoint with the OAuth bearer token
    - Confirm response parsing handles standard quota / reset fields
    - _Requirements: 5.1_

- [x] 4. P0 Checkpoint
  - Run `cargo test codex_desktop --lib`, `cargo test commands::provider --lib`, `cargo test codex_oauth_auth --lib`
  - Confirm all P0 tasks (1.x, 2.x, 3.x) pass; ask the user before moving to P1.

### P1 — Should Complete (quality of life)

- [x] 5. Capability-aware quota query for third-party providers
  - [x] 5.1 Introduce `UsageResult::unsupported` semantic
    - In the provider service chain (likely `services/provider/usage.rs` or `commands/provider.rs`), define a way to express "quota query not supported" that is NOT an error: e.g., `UsageResult { success: true, data: None, error: None, unsupported: Some(true) }` or a dedicated variant
    - Ensure this state propagates to the `usage-cache-updated` event payload
    - _Requirements: 5.2, 5.3_

  - [x] 5.2 Return `unsupported` for third-party providers without usage script
    - In `query_provider_usage_inner()`, when a Codex API Key provider has no `usage_script` and no recognized `template_type`, return `unsupported` instead of `Err`
    - Ensure switching the provider remains possible regardless of this state
    - _Requirements: 5.2, 5.3_

  - [x] 5.3 Frontend displays connection status and unsupported state
    - In the usage display component(s), render `unsupported` as a neutral "无法查询额度 / 接口不支持" indicator (not red/failure)
    - For third-party providers, prefer showing a connection status (live or last-checked) when available
    - _Requirements: 5.2_

  - [x] 5.4 Isolate quota query failures from provider state
    - Confirm a failed usage query does NOT modify the provider DB record
    - Confirm `switchProvider` works regardless of last usage query outcome
    - _Requirements: 5.3_

- [x] 6. P1 Checkpoint
  - Verify P1 tasks pass; gather user feedback on third-party UX before P2.

### P2 — Nice to Have (hardening)

- [ ] 7. End-to-end regression coverage
  - [ ] 7.1 Round-trip test: add API Key provider → switch → verify config.toml updated
    - Use a temp HOME and a fake API key
    - Assert `~/.codex/config.toml` contains the third-party `base_url` after switch
    - _Requirements: 3.1, 3.2_

  - [ ] 7.2 Round-trip test: switch back to OAuth provider after API Key
    - Assert OAuth `auth.json` is restored correctly
    - _Requirements: 3.1, 3.3_

  - [ ] 7.3 Old account compatibility test
    - Create a Provider JSON missing `meta.providerType` and `meta.authBinding`
    - Assert it deserializes without panic and is classified as API Key
    - _Requirements: 6.1, 6.2, 6.3, 6.4_

- [ ] 8. Database atomic operation hardening
  - [ ] 8.1 Verify `set_current_provider()` and `save_provider()` atomicity
    - Confirm a failed switch does not leave `is_current` pointing to a non-existent provider
    - Confirm rollback path restores prior current-provider marker
    - _Requirements: Security 4.4_

- [ ] 9. Log and key audit
  - [ ] 9.1 Audit log statements across the provider service chain
    - Files: `services/codex_desktop.rs`, `proxy/providers/codex_oauth_auth.rs`, `commands/provider.rs`, `services/provider/usage.rs`
    - Ensure no `log::*` call prints `OPENAI_API_KEY`, OAuth tokens, or refresh tokens
    - _Requirements: Security 4.1_

  - [ ] 9.2 Verify `.gitignore` covers sensitive files
    - Confirm `.gitignore` excludes `~/.codex/auth.json`-style local files and any test fixtures with real keys
    - Confirm test fixtures use obviously-fake placeholders (e.g., `sk-test-...`)
    - _Requirements: Security 4.2_

- [ ] 10. Final P2 Checkpoint
  - Run full test suite; confirm acceptance gates below.

## Acceptance Gates (must all pass)

- [ ] Add official account succeeds
- [ ] Add third-party API Key provider succeeds
- [ ] Added accounts immediately visible in list
- [ ] Official account can query quota
- [ ] Third-party account quota `unsupported` is NOT a failure
- [ ] Switching third-party Codex provider does NOT read OAuth snapshot
- [ ] `~/.codex/config.toml` is truly updated after switch
- [ ] Switch failure does not crash
- [ ] Logs do not contain raw keys or tokens

## Notes

- Tasks marked with `*` would be optional, but this plan currently has none — the P0/P1/P2 split itself encodes priority
- All P0 tasks are release blockers; P1 improves UX; P2 hardens long-term safety
- File references describe the **service chain**, not single files — reviewers should expect changes across `services/`, `commands/`, and frontend hooks
- No MiMo / DeepSeek / LiteLLM scope expansion in this plan
- Run Rust tests with: `cargo test codex_desktop --lib` and `cargo test --lib`
- Run frontend type checks with: `pnpm typecheck`

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["1.2", "1.3"] },
    { "id": 2, "tasks": ["1.4", "2.1", "2.2", "2.3", "2.4", "3.1", "3.2"] },
    { "id": 3, "tasks": ["4"] },
    { "id": 4, "tasks": ["5.1"] },
    { "id": 5, "tasks": ["5.2", "5.3", "5.4"] },
    { "id": 6, "tasks": ["6"] },
    { "id": 7, "tasks": ["7.1", "7.2", "7.3", "8.1", "9.1", "9.2"] },
    { "id": 8, "tasks": ["10"] }
  ]
}
```
