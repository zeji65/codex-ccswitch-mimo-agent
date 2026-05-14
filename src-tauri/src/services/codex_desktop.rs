use crate::app_config::AppType;
use crate::config::{atomic_write, delete_file, get_home_dir};
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::providers::codex_oauth_auth::CodexOAuthManager;
use crate::services::{provider::write_live_with_common_config, McpService};
use crate::settings;
use crate::store::AppState;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

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

#[allow(dead_code)]
pub fn switch_desktop_to_provider(
    state: &AppState,
    provider: &Provider,
    original_provider: Option<&Provider>,
    set_current_provider: bool,
) -> Result<(), AppError> {
    let (resolved_auth, _) = crate::services::provider::resolve_codex_live_parts(provider)?;
    let _target_account_id = read_account_id_from_auth_value(&resolved_auth)
        .ok_or_else(|| AppError::Message("无法从目标 Codex 登录态中解析 account_id".to_string()))?;

    let previous_current_provider_id =
        settings::get_effective_current_provider(&state.db, &AppType::Codex)?;

    stop_codex_processes()?;
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

        write_live_with_common_config(state.db.as_ref(), &AppType::Codex, provider)?;
        McpService::sync_all_enabled(state)?;
        launch_codex_app()?;
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

        if let Err(restore_err) = launch_codex_app() {
            log::warn!("[CodexDesktop] Failed to relaunch Codex after rollback: {restore_err}");
        }

        return Err(err);
    }

    Ok(())
}

pub async fn switch_desktop_to_provider_with_runtime_auth_sync(
    state: &AppState,
    provider: &Provider,
    original_provider: Option<&Provider>,
    set_current_provider: bool,
) -> Result<(), AppError> {
    let (resolved_auth, _) = crate::services::provider::resolve_codex_live_parts(provider)?;
    let _target_account_id = read_account_id_from_auth_value(&resolved_auth)
        .ok_or_else(|| AppError::Message("无法从目标 Codex 登录态中解析 account_id".to_string()))?;

    let previous_current_provider_id =
        settings::get_effective_current_provider(&state.db, &AppType::Codex)?;

    // Do not kill/relaunch Codex from the account-pool switch path. In practice
    // users may operate CC Switch from Codex Desktop itself, and killing every
    // Codex.app/Codex Helper process closes that host app.
    sync_tracked_runtime_auth_after_stop().await;
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

        write_live_with_common_config(state.db.as_ref(), &AppType::Codex, provider)?;
        McpService::sync_all_enabled(state)?;
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

        return Err(err);
    }

    Ok(())
}

async fn sync_tracked_runtime_auth_after_stop() {
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

#[cfg(target_os = "macos")]
#[allow(dead_code)]
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

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
pub fn launch_codex_app() -> Result<(), AppError> {
    Err(AppError::Message(
        "Codex 桌面账号切换当前仅支持 macOS".to_string(),
    ))
}

pub fn restart_codex_app() -> Result<(), AppError> {
    stop_codex_processes()?;
    launch_codex_app()
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

#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn find_codex_app_path() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("/Applications/Codex.app"),
        get_home_dir().join("Applications/Codex.app"),
    ];

    candidates.into_iter().find(|path| path.exists())
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
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

#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn is_codex_process_command(command: &str) -> bool {
    command.contains("Codex.app/Contents/MacOS/Codex")
        || command.contains("Codex.app/Contents/Frameworks/Codex Helper")
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
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

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn stop_codex_processes() -> Result<(), AppError> {
    Err(AppError::Message(
        "Codex 桌面账号切换当前仅支持 macOS".to_string(),
    ))
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

    #[cfg(target_os = "macos")]
    #[test]
    fn codex_process_detection_matches_main_and_helper_processes() {
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
            "/Users/example/.vscode/extensions/openai.chatgpt/bin/macos-aarch64/codex app-server --analytics-default-enabled"
        ));
        assert!(!is_codex_process_command(
            "/Applications/Codex.app/Contents/Resources/codex app-server --analytics-default-enabled"
        ));
        assert!(!is_codex_process_command(
            "/Applications/Other.app/Contents/MacOS/Other"
        ));
    }
}
