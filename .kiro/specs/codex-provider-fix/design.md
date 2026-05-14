# Design Document

## Overview

This design addresses the root cause crash in Codex Desktop provider switching: `build_effective_codex_settings()` unconditionally calls `build_native_auth()` for providers that happen to have `requires_openai_auth = true` in their TOML config but are not actual ChatGPT OAuth providers. The fix introduces explicit auth type detection based on provider metadata (`meta.providerType` and `meta.authBinding`) rather than TOML config fields, and updates validation to accept the API Key auth format used by third-party relay stations.

## Architecture

The fix is contained within the existing `codex_desktop.rs` service module. No new modules or services are introduced. The change modifies two functions:

1. **`build_effective_codex_settings()`** — Add a guard that checks provider metadata before invoking OAuth snapshot generation.
2. **`validate_target_auth()`** — Expand validation to accept both OAuth (`auth_mode: "chatgpt"`) and API Key (`OPENAI_API_KEY`) auth formats.

The detection logic leverages the existing `Provider::is_codex_oauth()` method and `ProviderMeta::managed_account_id_for()` helper, which already correctly distinguish OAuth providers from API Key providers based on metadata.

## Components and Interfaces

### Auth Type Detection (Modified)

**Location:** `src-tauri/src/services/codex_desktop.rs` → `build_effective_codex_settings()`

The current implementation:
```rust
fn build_effective_codex_settings(provider: &Provider) -> Result<Value, AppError> {
    let mut settings = provider.settings_config.clone();
    let managed_account_id = provider.meta.as_ref()
        .and_then(|meta| meta.managed_account_id_for(CODEX_OAUTH_AUTH_PROVIDER));
    let is_codex_oauth = provider.is_codex_oauth();

    if is_codex_oauth || managed_account_id.is_some() {
        // OAuth path: calls build_native_auth()
    }
    Ok(settings)
}
```

The existing guard (`is_codex_oauth || managed_account_id.is_some()`) is already correct in its logic — `is_codex_oauth()` checks `meta.provider_type == "codex_oauth"` and `managed_account_id_for()` checks `meta.authBinding`. The bug is that some providers reach this function with metadata that triggers these checks when they shouldn't.

**Fix approach:** The detection logic itself is sound. The issue is that providers created via the Universal Provider system (`to_codex_provider()`) inherit the parent's `meta` field, which may contain an `authBinding` from a different context. The fix ensures that:
1. API Key providers that lack both `meta.providerType == "codex_oauth"` and a `codex_oauth` auth binding skip the OAuth path entirely (already correct).
2. When the OAuth path is skipped, the existing `auth` field from `settings_config` is passed through unchanged.
3. When `settings_config` lacks an `auth` field for an API Key provider, a minimal auth object is constructed from the `OPENAI_API_KEY` in the config TOML.

### Auth Validation (Modified)

**Location:** `src-tauri/src/services/codex_desktop.rs` → `validate_target_auth()`

The current implementation only validates the `auth_mode: "chatgpt"` format and rejects settings without an `auth` field entirely. The fix:

```rust
fn validate_target_auth(settings: &Value) -> Result<(), AppError> {
    let Some(auth) = settings.get("auth") else {
        return Err(AppError::Config(
            "Codex 供应商配置缺少 'auth' 字段".to_string(),
        ));
    };

    // API Key format: { "OPENAI_API_KEY": "sk-..." }
    if auth.get("OPENAI_API_KEY").and_then(Value::as_str).is_some() {
        return Ok(()); // Valid API Key auth
    }

    // OAuth format: { "auth_mode": "chatgpt", "tokens": { "account_id": "..." } }
    if auth.get("auth_mode").and_then(Value::as_str) == Some("chatgpt") {
        if read_account_id_from_auth_value(auth).is_none() {
            return Err(AppError::Message(
                "无法从目标 Codex 登录态中解析 account_id".to_string(),
            ));
        }
        return Ok(());
    }

    // Unknown format but has auth field — allow passthrough
    Ok(())
}
```

### Fallback Auth Construction (New)

**Location:** `src-tauri/src/services/codex_desktop.rs` → `build_effective_codex_settings()`

When an API Key provider's `settings_config` lacks a top-level `auth` field but contains `OPENAI_API_KEY` elsewhere (e.g., in the TOML config string), the function constructs a minimal auth object:

```rust
// After the OAuth guard, for API Key providers:
if settings.get("auth").is_none() {
    // Try to extract OPENAI_API_KEY from config TOML or other settings fields
    if let Some(api_key) = extract_api_key_from_settings(&settings) {
        let obj = settings.as_object_mut()
            .ok_or_else(|| AppError::Config("settings must be JSON object".to_string()))?;
        obj.insert("auth".to_string(), serde_json::json!({
            "OPENAI_API_KEY": api_key
        }));
    }
}
```

### Helper: API Key Extraction

```rust
fn extract_api_key_from_settings(settings: &Value) -> Option<String> {
    // Direct auth.OPENAI_API_KEY (most common for relay stations)
    if let Some(key) = settings.pointer("/auth/OPENAI_API_KEY").and_then(Value::as_str) {
        return Some(key.to_string());
    }
    // Fallback: check if config TOML references an API key pattern
    // (Universal providers store it in settings_config.auth.OPENAI_API_KEY)
    None
}
```

## Data Models

No schema changes. The fix relies on existing metadata fields:

| Field | Location | Purpose |
|-------|----------|---------|
| `meta.providerType` | `ProviderMeta.provider_type` | `"codex_oauth"` indicates OAuth provider |
| `meta.authBinding.source` | `ProviderMeta.auth_binding.source` | `ManagedAccount` indicates managed auth |
| `meta.authBinding.authProvider` | `ProviderMeta.auth_binding.auth_provider` | `"codex_oauth"` indicates OAuth binding |
| `settings_config.auth.OPENAI_API_KEY` | Provider JSON | API Key for relay stations |
| `settings_config.auth.auth_mode` | Provider JSON | `"chatgpt"` for OAuth providers |

### Provider Classification Truth Table

| `meta.providerType` | `meta.authBinding` (codex_oauth) | Classification | Auth Behavior |
|---------------------|----------------------------------|----------------|---------------|
| `"codex_oauth"` | any | OAuth_Provider | Call `build_native_auth()` |
| other/absent | `source=managed_account, authProvider=codex_oauth` | OAuth_Provider | Call `build_native_auth()` |
| other/absent | absent/other | API_Key_Provider | Passthrough `settings_config.auth` |

## Error Handling

| Scenario | Error Type | Message | Recovery |
|----------|-----------|---------|----------|
| OAuth provider, no snapshot | `AppError::Message` | "生成 Codex 原生登录态失败: 无可用的 ChatGPT 账号" | User must complete OAuth login |
| OAuth provider, snapshot mismatch | `AppError::Message` | "生成 Codex 原生登录态失败: ..." | Re-import account |
| API Key provider, missing auth field | `AppError::Config` | "Codex 供应商配置缺少 'auth' 字段" | User must add API key |
| OAuth auth without account_id | `AppError::Message` | "无法从目标 Codex 登录态中解析 account_id" | Re-login or re-import |
| Settings not a JSON object | `AppError::Config` | "Codex 供应商配置必须是 JSON 对象" | Fix provider config |
| Process termination failure | `AppError::Message` | "无法完全停止 Codex.app..." | Manual process kill |

All errors from `build_effective_codex_settings()` and `validate_target_auth()` are caught by the existing rollback mechanism in `switch_desktop_to_provider_inner()`, which restores runtime files, database state, and current provider markers.

## Testing Strategy

### Unit Tests
- Auth type classification: verify `is_codex_oauth()` and `managed_account_id_for()` return correct values for various metadata combinations
- `validate_target_auth()`: verify acceptance of API Key format, OAuth format with account_id, and rejection of OAuth format without account_id
- `build_effective_codex_settings()`: verify OAuth path is taken only for OAuth providers, API Key passthrough for others
- Fallback auth construction: verify OPENAI_API_KEY extraction when auth field is missing
- Provider deserialization: verify JSON with missing optional fields deserializes correctly

### Property Tests
- Classification completeness (Property 1): generate random Provider structs, verify classification matches metadata
- Auth passthrough (Property 2): generate random API Key providers, verify auth field preservation
- Validation biconditional (Property 3): generate random auth objects, verify acceptance/rejection matches rules
- Fallback construction (Property 4): generate API Key providers without auth, verify construction
- Deserialization robustness (Property 5): generate minimal JSON objects, verify successful deserialization

### Integration Tests
- End-to-end switch with API Key provider: verify runtime files are written correctly
- End-to-end switch with OAuth provider: verify OAuth snapshot is used
- Rollback on failure: verify runtime files restored after build_effective_codex_settings failure

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system — essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: Auth Type Classification Completeness

*For any* Provider struct, the auth type detection classifies it as an OAuth_Provider if and only if `meta.providerType == "codex_oauth"` OR (`meta.authBinding.source == ManagedAccount` AND `meta.authBinding.authProvider == "codex_oauth"`). All other providers are classified as API_Key_Provider regardless of their TOML config content.

**Validates: Requirements 1.1, 1.2, 1.3, 6.1, 6.2, 6.3**

### Property 2: API Key Auth Passthrough Preservation

*For any* Provider classified as an API_Key_Provider that has an `auth` field in its `settings_config`, calling `build_effective_codex_settings()` SHALL produce effective settings where the `auth` field is identical to the original `settings_config.auth` value.

**Validates: Requirements 1.4**

### Property 3: Validation Acceptance Biconditional

*For any* effective settings JSON with an `auth` field, `validate_target_auth()` returns Ok if and only if: (a) `auth.OPENAI_API_KEY` is a non-null string, OR (b) `auth.auth_mode == "chatgpt"` AND `auth.tokens.account_id` is a non-null string, OR (c) the auth object has neither `OPENAI_API_KEY` nor `auth_mode == "chatgpt"` (unknown format passthrough).

**Validates: Requirements 2.1, 2.2**

### Property 4: Fallback Auth Construction

*For any* Provider classified as an API_Key_Provider whose `settings_config` lacks a top-level `auth` field but contains an extractable `OPENAI_API_KEY` value, `build_effective_codex_settings()` SHALL produce effective settings containing `auth.OPENAI_API_KEY` equal to the extracted key value.

**Validates: Requirements 2.3**

### Property 5: Provider Deserialization Robustness

*For any* JSON object containing at minimum the required Provider fields (`id`, `name`, `settingsConfig`), deserializing into a Provider struct SHALL succeed with appropriate defaults for all missing optional fields, and the resulting Provider SHALL be classifiable by the auth type detection logic without panicking.

**Validates: Requirements 6.4**
