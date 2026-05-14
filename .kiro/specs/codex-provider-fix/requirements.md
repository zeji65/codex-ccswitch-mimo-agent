# Requirements Document

## Introduction

This feature fixes the Codex Desktop provider switching in CC Switch to correctly distinguish between official OAuth providers and third-party API Key providers. The root cause is that `build_effective_codex_settings()` unconditionally invokes `build_native_auth()` for providers that have `requires_openai_auth = true` in their config but are not actual ChatGPT OAuth providers. Third-party relay stations (e.g., tcdmx.com, aihubmix.com) use API Key authentication and should never trigger OAuth snapshot generation. Additionally, `validate_target_auth()` requires an `auth` field that API Key providers structure differently from OAuth providers, causing crashes on switch.

## Glossary

- **CC_Switch**: The Tauri-based desktop application that manages provider switching for Codex Desktop, Claude Desktop, and other AI coding tools.
- **Codex_Desktop**: The OpenAI Codex Desktop application whose runtime configuration files (`~/.codex/auth.json` and `~/.codex/config.toml`) are managed by CC_Switch.
- **Provider**: A configured AI service endpoint stored in CC_Switch's database, containing settings, metadata, and authentication information.
- **OAuth_Provider**: A Provider whose `meta.providerType` equals `"codex_oauth"` or whose `meta.authBinding` references the `codex_oauth` managed auth provider. Uses ChatGPT Plus/Pro subscription OAuth Device Code flow.
- **API_Key_Provider**: A Provider that authenticates via an `OPENAI_API_KEY` field in its settings config. Third-party relay stations and aggregators fall into this category.
- **Auth_Type_Detection**: The logic in `build_effective_codex_settings()` that determines whether a Provider requires OAuth snapshot generation or API Key passthrough.
- **OAuth_Snapshot**: A persisted copy of ChatGPT OAuth tokens (access_token, refresh_token, account_id) stored by `CodexOAuthManager` for a managed account.
- **Switch_Transaction**: The atomic operation in `switch_desktop_to_provider_inner()` that backs up runtime files, writes new config, syncs MCP, and relaunches Codex Desktop with rollback on failure.
- **Relay_Station**: A third-party service that proxies requests to OpenAI APIs using its own infrastructure, requiring only an API Key from the user.

## Requirements

### Requirement 1: Auth Type Detection

**User Story:** As a CC_Switch user, I want the provider switch logic to correctly identify whether a Codex provider uses OAuth or API Key authentication, so that switching to a third-party relay station does not attempt OAuth snapshot generation.

#### Acceptance Criteria

1. WHEN a Provider has `meta.providerType` equal to `"codex_oauth"`, THE Auth_Type_Detection SHALL classify the Provider as an OAuth_Provider.
2. WHEN a Provider has `meta.authBinding` with `source` equal to `"managed_account"` and `authProvider` equal to `"codex_oauth"`, THE Auth_Type_Detection SHALL classify the Provider as an OAuth_Provider.
3. WHEN a Provider has neither `meta.providerType` equal to `"codex_oauth"` nor a `codex_oauth` managed auth binding, THE Auth_Type_Detection SHALL classify the Provider as an API_Key_Provider regardless of the `requires_openai_auth` value in its TOML config.
4. WHEN a Provider is classified as an API_Key_Provider, THE CC_Switch SHALL skip OAuth_Snapshot generation and pass through the existing `auth` field from `settings_config` without modification.

### Requirement 2: API Key Auth Format Acceptance

**User Story:** As a CC_Switch user, I want the switch validation to accept the API Key authentication format used by third-party providers, so that switching to relay stations succeeds without errors.

#### Acceptance Criteria

1. WHEN a Provider's effective settings contain an `auth` field with `OPENAI_API_KEY` as a top-level key, THE CC_Switch SHALL accept the auth format as valid for switching.
2. WHEN a Provider's effective settings contain an `auth` field with `auth_mode` equal to `"chatgpt"`, THE CC_Switch SHALL require a valid `tokens.account_id` in the auth structure.
3. WHEN a Provider's effective settings lack an `auth` field and the Provider is classified as an API_Key_Provider, THE CC_Switch SHALL construct a minimal auth object from the Provider's `settings_config` containing the `OPENAI_API_KEY` value.
4. WHEN a Provider's effective settings lack an `auth` field and the Provider is classified as an OAuth_Provider, THE CC_Switch SHALL return a descriptive error indicating the OAuth snapshot is missing.

### Requirement 3: Switch Failure Graceful Handling

**User Story:** As a CC_Switch user, I want provider switch failures to return actionable error messages instead of crashing the application, so that I can understand what went wrong and take corrective action.

#### Acceptance Criteria

1. IF `build_effective_codex_settings()` fails for any reason, THEN THE CC_Switch SHALL return an `AppError` with a descriptive message and restore the previous runtime state via the existing rollback mechanism.
2. IF `validate_target_auth()` fails, THEN THE CC_Switch SHALL return an `AppError` describing the validation failure without modifying any runtime files or database state.
3. IF an OAuth_Snapshot is missing for an OAuth_Provider during switch, THEN THE CC_Switch SHALL return an error message indicating the user needs to complete OAuth login before switching.
4. IF Codex_Desktop process termination fails, THEN THE CC_Switch SHALL return an error message suggesting manual process termination without leaving runtime files in an inconsistent state.

### Requirement 4: Add-Account Capability Restoration

**User Story:** As a CC_Switch user, I want to add both official ChatGPT OAuth accounts and third-party API Key accounts to CC_Switch for Codex Desktop, so that I can manage multiple providers.

#### Acceptance Criteria

1. THE CC_Switch SHALL support adding OAuth_Providers through the existing `auth_start_login` and `auth_poll_for_account` device code flow for the `codex_oauth` auth provider.
2. THE CC_Switch SHALL support adding API_Key_Providers by accepting a Provider with `OPENAI_API_KEY` in its `settings_config.auth` field without requiring OAuth device code flow.
3. WHEN an API_Key_Provider is added, THE CC_Switch SHALL store the Provider in the database without creating an OAuth_Snapshot or managed auth binding.
4. WHEN an OAuth_Provider is added and the device code flow completes, THE CC_Switch SHALL persist the OAuth_Snapshot and create a managed auth binding in the Provider's metadata.

### Requirement 5: Quota and Status Query

**User Story:** As a CC_Switch user, I want to see account status information for my Codex providers, so that I can monitor my usage and connection health.

#### Acceptance Criteria

1. WHEN an OAuth_Provider has a valid OAuth_Snapshot, THE CC_Switch SHALL display balance or subscription reset information through the existing usage script mechanism.
2. WHEN an API_Key_Provider is queried for status, THE CC_Switch SHALL display connection status or a "not supported" indicator if the relay station does not expose a usage API.
3. IF a usage query fails for an API_Key_Provider, THEN THE CC_Switch SHALL display the failure reason without affecting the Provider's switch capability.

### Requirement 6: Database Compatibility

**User Story:** As a CC_Switch user upgrading from an older version, I want my existing account data to be read correctly by the new version, so that I do not lose my configured providers.

#### Acceptance Criteria

1. WHEN the CC_Switch reads a Provider record that lacks the `meta.authBinding` field, THE CC_Switch SHALL treat the Provider as an API_Key_Provider by default unless `meta.providerType` equals `"codex_oauth"`.
2. WHEN the CC_Switch reads a Provider record that lacks the `meta.providerType` field, THE CC_Switch SHALL infer the auth type from the presence of `meta.authBinding` with `authProvider` equal to `"codex_oauth"`.
3. WHEN both `meta.providerType` and `meta.authBinding` are absent, THE CC_Switch SHALL treat the Provider as an API_Key_Provider and use the `OPENAI_API_KEY` from `settings_config` for authentication.
4. THE CC_Switch SHALL read Provider records from older database versions without requiring a schema migration, using default values for any missing fields.
