use crate::app_config::AppType;
use crate::config::{atomic_write, delete_file};
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::providers::codex_oauth_auth::CodexOAuthManager;
use crate::services::{provider::write_live_with_common_config, McpService};
use crate::settings;
use crate::store::AppState;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[cfg(all(target_os = "macos", not(test)))]
use crate::config::get_home_dir;
#[cfg(all(target_os = "macos", not(test)))]
use std::path::PathBuf;
#[cfg(all(target_os = "macos", not(test)))]
use std::process::Command;
#[cfg(all(target_os = "macos", not(test)))]
use std::thread;
#[cfg(all(target_os = "macos", not(test)))]
use std::time::Duration;

const CODEX_OAUTH_AUTH_PROVIDER: &str = "codex_oauth";

#[derive(Debug, Clone)]
pub struct CodexRuntimeBackup {
    auth_bytes: Option<Vec<u8>>,
    config_bytes: Option<Vec<u8>>,
}

pub fn read_current_codex_auth_account_id() -> Result<Option<String>, AppError> {
    let auth_path = crate::codex_config::get_codex_auth_path();
    if !auth_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&auth_path).map_err(|e| AppError::io(&auth_path, e))?;
    let value: Value = serde_json::from_str(&content).map_err(|e| AppError::json(&auth_path, e))?;
    Ok(read_account_id_from_auth_value(&value))
}

pub fn backup_runtime_files() -> Result<CodexRuntimeBackup, AppError> {
    Ok(CodexRuntimeBackup {
        auth_bytes: read_optional_file(&crate::codex_config::get_codex_auth_path())?,
        config_bytes: read_optional_file(&crate::codex_config::get_codex_config_path())?,
    })
}

pub fn restore_runtime_files(backup: &CodexRuntimeBackup) -> Result<(), AppError> {
    restore_optional_file(
        &crate::codex_config::get_codex_auth_path(),
        backup.auth_bytes.as_deref(),
    )?;
    restore_optional_file(
        &crate::codex_config::get_codex_config_path(),
        backup.config_bytes.as_deref(),
    )?;
    Ok(())
}

pub fn switch_desktop_to_provider(
    state: &AppState,
    provider: &Provider,
    original_provider: Option<&Provider>,
    set_current_provider: bool,
) -> Result<(), AppError> {
    stop_codex_processes()?;
    switch_desktop_to_provider_inner(
        state,
        provider,
        original_provider,
        set_current_provider,
        true,
    )
}

pub fn restart_codex_app() -> Result<(), AppError> {
    stop_codex_processes()?;
    launch_codex_app()
}

pub async fn switch_desktop_to_provider_with_runtime_auth_sync(
    state: &AppState,
    provider: &Provider,
    original_provider: Option<&Provider>,
    set_current_provider: bool,
) -> Result<(), AppError> {
    sync_tracked_runtime_auth_before_switch().await;
    switch_desktop_to_provider_inner(
        state,
        provider,
        original_provider,
        set_current_provider,
        false,
    )
}

fn switch_desktop_to_provider_inner(
    state: &AppState,
    provider: &Provider,
    original_provider: Option<&Provider>,
    set_current_provider: bool,
    relaunch_on_success: bool,
) -> Result<(), AppError> {
    let effective_settings = build_effective_codex_settings(provider)?;
    validate_target_auth(&effective_settings)?;

    let previous_current_provider_id =
        settings::get_effective_current_provider(&state.db, &AppType::Codex)?;
    let runtime_backup = backup_runtime_files()?;

    let mut provider_saved = false;
    let mut current_provider_changed = false;

    let apply_result = (|| -> Result<(), AppError> {
        if let Some(original_provider) = original_provider {
            if original_provider.id != provider.id {
                return Err(AppError::Message(
                    "Codex 桌面切换当前仅支持更新当前 provider，不支持改写其他 provider"
                        .to_string(),
                ));
            }
            state.db.save_provider(AppType::Codex.as_str(), provider)?;
            provider_saved = true;
        }

        if set_current_provider {
            settings::set_current_provider(&AppType::Codex, Some(provider.id.as_str()))?;
            state
                .db
                .set_current_provider(AppType::Codex.as_str(), &provider.id)?;
            current_provider_changed = true;
        }

        let effective_provider = Provider {
            settings_config: effective_settings,
            ..provider.clone()
        };
        write_live_with_common_config(state.db.as_ref(), &AppType::Codex, &effective_provider)?;
        McpService::sync_all_enabled(state)?;
        if relaunch_on_success {
            launch_codex_app()?;
        }
        Ok(())
    })();

    if let Err(err) = apply_result {
        if provider_saved {
            if let Some(original_provider) = original_provider {
                if let Err(restore_err) = state
                    .db
                    .save_provider(AppType::Codex.as_str(), original_provider)
                {
                    log::warn!(
                        "[CodexDesktop] Failed to restore provider after switch error: {restore_err}"
                    );
                }
            }
        }

        if current_provider_changed {
            if let Err(restore_err) =
                restore_current_provider(state, previous_current_provider_id.as_deref())
            {
                log::warn!(
                    "[CodexDesktop] Failed to restore current provider marker after switch error: {restore_err}"
                );
            }
        }

        if let Err(restore_err) = restore_runtime_files(&runtime_backup) {
            log::warn!(
                "[CodexDesktop] Failed to restore runtime auth/config after switch error: {restore_err}"
            );
        }

        if relaunch_on_success {
            if let Err(restore_err) = launch_codex_app() {
                log::warn!("[CodexDesktop] Failed to relaunch Codex after rollback: {restore_err}");
            }
        }

        return Err(err);
    }

    Ok(())
}

fn build_effective_codex_settings(provider: &Provider) -> Result<Value, AppError> {
    let mut settings = provider.settings_config.clone();

    // Rule: API Key 优先，OAuth 明确 opt-in.
    // Only providers explicitly marked as codex_oauth (via meta.providerType or meta.authBinding)
    // should trigger OAuth snapshot generation. All others are treated as API Key providers.
    let managed_account_id = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.managed_account_id_for(CODEX_OAUTH_AUTH_PROVIDER));
    let is_codex_oauth = provider.is_codex_oauth();

    if is_codex_oauth || managed_account_id.is_some() {
        // OAuth path: only for providers with explicit codex_oauth metadata
        let manager = CodexOAuthManager::new(crate::config::get_app_config_dir());
        let auth =
            futures::executor::block_on(manager.build_native_auth(managed_account_id.as_deref()))
                .map_err(|err| AppError::Message(format!("生成 Codex 原生登录态失败: {err}")))?;
        let obj = settings
            .as_object_mut()
            .ok_or_else(|| AppError::Config("Codex 供应商配置必须是 JSON 对象".to_string()))?;
        obj.insert("auth".to_string(), auth);
    } else if settings.get("auth").is_none() {
        // API Key fallback: construct minimal auth from settings for Codex Desktop runtime.
        // This only applies to non-OAuth providers that lack an explicit auth field.
        if let Some(api_key) = extract_api_key_from_settings(&settings) {
            let obj = settings
                .as_object_mut()
                .ok_or_else(|| AppError::Config("Codex 供应商配置必须是 JSON 对象".to_string()))?;
            obj.insert(
                "auth".to_string(),
                serde_json::json!({ "OPENAI_API_KEY": api_key }),
            );
        }
    }

    Ok(settings)
}

/// Extract OPENAI_API_KEY from provider settings for API Key providers.
///
/// Checks common locations where the key might be stored in the settings JSON.
/// Returns None if no key is found (provider may need manual configuration).
fn extract_api_key_from_settings(settings: &Value) -> Option<String> {
    // Check direct auth.OPENAI_API_KEY (most common for relay station providers)
    if let Some(key) = settings
        .pointer("/auth/OPENAI_API_KEY")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(key.to_string());
    }

    // Check env.OPENAI_API_KEY (alternative location used by some provider configs)
    if let Some(key) = settings
        .pointer("/env/OPENAI_API_KEY")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(key.to_string());
    }

    None
}

fn validate_target_auth(settings: &Value) -> Result<(), AppError> {
    let Some(auth) = settings.get("auth") else {
        return Err(AppError::Config(
            "Codex 供应商配置缺少 'auth' 字段".to_string(),
        ));
    };

    // API Key format: { "OPENAI_API_KEY": "sk-..." } — valid for relay stations
    if auth
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .is_some()
    {
        return Ok(());
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

    // Unknown format but auth field exists — allow passthrough
    // (e.g., custom auth structures from third-party providers)
    Ok(())
}

pub async fn sync_tracked_runtime_auth_before_switch() {
    let manager = CodexOAuthManager::new(crate::config::get_app_config_dir());
    match manager.sync_current_native_auth_if_tracked().await {
        Ok(Some(account)) => {
            log::info!(
                "[CodexDesktop] 已在切换前同步当前 Codex 登录态快照: {}",
                account.id
            );
        }
        Ok(None) => {
            log::debug!("[CodexDesktop] 切换前未发现可同步的受管 Codex 登录态");
        }
        Err(err) => {
            log::debug!("[CodexDesktop] 切换前同步当前 Codex 登录态失败，继续切换: {err}");
        }
    }
}

fn read_account_id_from_auth_value(value: &Value) -> Option<String> {
    value
        .get("tokens")
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    if !path.exists() {
        return Ok(None);
    }

    fs::read(path).map(Some).map_err(|e| AppError::io(path, e))
}

fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<(), AppError> {
    match bytes {
        Some(bytes) => atomic_write(path, bytes),
        None => {
            if path.exists() {
                delete_file(path)?;
            }
            Ok(())
        }
    }
}

fn restore_current_provider(state: &AppState, provider_id: Option<&str>) -> Result<(), AppError> {
    settings::set_current_provider(&AppType::Codex, provider_id)?;
    if let Some(provider_id) = provider_id {
        state
            .db
            .set_current_provider(AppType::Codex.as_str(), provider_id)?;
    }
    Ok(())
}

#[cfg(all(target_os = "macos", not(test)))]
pub fn launch_codex_app() -> Result<(), AppError> {
    let app_path = find_codex_app_path()
        .ok_or_else(|| AppError::Message("未找到 Codex.app，请确认已安装桌面版".to_string()))?;

    let status = Command::new("open")
        .arg(&app_path)
        .status()
        .map_err(|e| AppError::io(&app_path, e))?;

    if !status.success() {
        return Err(AppError::Message(format!(
            "启动 Codex.app 失败，退出码: {:?}",
            status.code()
        )));
    }

    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
pub fn launch_codex_app() -> Result<(), AppError> {
    Ok(())
}

#[cfg(all(target_os = "macos", not(test)))]
fn find_codex_app_path() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("/Applications/Codex.app"),
        get_home_dir().join("Applications/Codex.app"),
    ];

    candidates.into_iter().find(|path| path.exists())
}

#[cfg(all(target_os = "macos", not(test)))]
fn list_running_codex_pids() -> Result<Vec<i32>, AppError> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .map_err(|e| AppError::Message(format!("读取 Codex 进程列表失败: {e}")))?;

    if !output.status.success() {
        return Err(AppError::Message("读取 Codex 进程列表失败".to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.splitn(2, ' ');
        let Some(pid_raw) = parts.next() else {
            continue;
        };
        let command = parts.next().unwrap_or("").trim();

        if !is_codex_process_command(command) {
            continue;
        }

        if let Ok(pid) = pid_raw.trim().parse::<i32>() {
            pids.push(pid);
        }
    }

    Ok(pids)
}

fn is_codex_process_command(command: &str) -> bool {
    let normalized = command.replace('\\', "/");
    normalized.ends_with("/Codex.app/Contents/MacOS/Codex")
        || normalized.contains("/Codex.app/Contents/Frameworks/Codex Helper.app/")
}

#[cfg(all(target_os = "macos", not(test)))]
fn stop_codex_processes() -> Result<(), AppError> {
    let initial_pids = list_running_codex_pids()?;
    if initial_pids.is_empty() {
        return Ok(());
    }

    for pid in &initial_pids {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }

    for _ in 0..15 {
        thread::sleep(Duration::from_millis(200));
        if list_running_codex_pids()?.is_empty() {
            return Ok(());
        }
    }

    for pid in list_running_codex_pids()? {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
    }

    for _ in 0..10 {
        thread::sleep(Duration::from_millis(150));
        if list_running_codex_pids()?.is_empty() {
            return Ok(());
        }
    }

    Err(AppError::Message(
        "无法完全停止 Codex.app，请先手动退出后再重试".to_string(),
    ))
}

#[cfg(any(not(target_os = "macos"), test))]
fn stop_codex_processes() -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_account_id_from_auth_value_extracts_nested_account_id() {
        let value = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "account_id": "acc-123"
            }
        });

        assert_eq!(
            read_account_id_from_auth_value(&value).as_deref(),
            Some("acc-123")
        );
    }

    #[test]
    fn read_account_id_from_auth_value_handles_missing_tokens() {
        let value = json!({ "auth_mode": "chatgpt" });
        assert_eq!(read_account_id_from_auth_value(&value), None);
    }

    #[test]
    fn codex_process_detection_matches_main_and_helper_processes_only() {
        assert!(is_codex_process_command(
            "/Applications/Codex.app/Contents/MacOS/Codex"
        ));
        assert!(is_codex_process_command(
            "/Users/example/Applications/Codex.app/Contents/MacOS/Codex"
        ));
        assert!(is_codex_process_command(
            "/Applications/Codex.app/Contents/Frameworks/Codex Helper.app/Contents/MacOS/Codex Helper --type=gpu-process"
        ));
        assert!(!is_codex_process_command(
            "/Applications/Codex.app/Contents/Resources/codex app-server"
        ));
        assert!(!is_codex_process_command(
            "/Applications/Other.app/Contents/MacOS/Other"
        ));
    }

    // ==================== Task 1.4: Auth detection and validation tests ====================

    #[test]
    fn extract_api_key_from_settings_finds_key_in_auth_field() {
        let settings = json!({
            "auth": { "OPENAI_API_KEY": "sk-test-key-123" },
            "config": "model_provider = \"OpenAI\""
        });
        assert_eq!(
            extract_api_key_from_settings(&settings).as_deref(),
            Some("sk-test-key-123")
        );
    }

    #[test]
    fn extract_api_key_from_settings_finds_key_in_env_field() {
        let settings = json!({
            "env": { "OPENAI_API_KEY": "sk-env-key-456" },
            "config": "model_provider = \"OpenAI\""
        });
        assert_eq!(
            extract_api_key_from_settings(&settings).as_deref(),
            Some("sk-env-key-456")
        );
    }

    #[test]
    fn extract_api_key_from_settings_returns_none_for_oauth_provider() {
        let settings = json!({
            "auth": { "auth_mode": "chatgpt", "tokens": { "account_id": "acc-1" } }
        });
        assert_eq!(extract_api_key_from_settings(&settings), None);
    }

    #[test]
    fn extract_api_key_from_settings_returns_none_for_empty_settings() {
        let settings = json!({});
        assert_eq!(extract_api_key_from_settings(&settings), None);
    }

    #[test]
    fn extract_api_key_from_settings_rejects_empty_string_key() {
        let settings = json!({
            "auth": { "OPENAI_API_KEY": "" }
        });
        assert_eq!(extract_api_key_from_settings(&settings), None);
    }

    #[test]
    fn validate_target_auth_accepts_api_key_format() {
        let settings = json!({
            "auth": { "OPENAI_API_KEY": "sk-test-key" },
            "config": "model_provider = \"OpenAI\""
        });
        assert!(validate_target_auth(&settings).is_ok());
    }

    #[test]
    fn validate_target_auth_accepts_oauth_format_with_account_id() {
        let settings = json!({
            "auth": {
                "auth_mode": "chatgpt",
                "tokens": { "account_id": "acc-123" }
            }
        });
        assert!(validate_target_auth(&settings).is_ok());
    }

    #[test]
    fn validate_target_auth_rejects_oauth_format_without_account_id() {
        let settings = json!({
            "auth": { "auth_mode": "chatgpt" }
        });
        let err = validate_target_auth(&settings).unwrap_err();
        assert!(err.to_string().contains("account_id"));
    }

    #[test]
    fn validate_target_auth_accepts_unknown_auth_format_passthrough() {
        let settings = json!({
            "auth": { "custom_field": "some_value" }
        });
        assert!(validate_target_auth(&settings).is_ok());
    }

    #[test]
    fn validate_target_auth_rejects_missing_auth_field() {
        let settings = json!({
            "config": "model_provider = \"OpenAI\""
        });
        let err = validate_target_auth(&settings).unwrap_err();
        assert!(err.to_string().contains("auth"));
    }
}
