use tauri::State;

use crate::app_config::AppType;
use crate::commands::codex_oauth::CodexOAuthState;
use crate::commands::copilot::CopilotAuthState;
use crate::provider::{AuthBinding, AuthBindingSource, Provider};
use crate::proxy::providers::codex_oauth_auth::CodexOAuthError;
use crate::proxy::providers::copilot_auth::{
    CopilotAuthError, GitHubAccount, GitHubDeviceCodeResponse,
};
use crate::store::AppState;

const AUTH_PROVIDER_GITHUB_COPILOT: &str = "github_copilot";
const AUTH_PROVIDER_CODEX_OAUTH: &str = "codex_oauth";

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedAuthAccount {
    pub id: String,
    pub provider: String,
    pub login: String,
    pub avatar_url: Option<String>,
    pub authenticated_at: i64,
    pub is_default: bool,
    pub github_domain: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedAuthStatus {
    pub provider: String,
    pub authenticated: bool,
    pub default_account_id: Option<String>,
    pub current_account_id: Option<String>,
    pub current_account_login: Option<String>,
    pub migration_error: Option<String>,
    pub accounts: Vec<ManagedAuthAccount>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedAuthDeviceCodeResponse {
    pub provider: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

fn ensure_auth_provider(auth_provider: &str) -> Result<&'static str, String> {
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => Ok(AUTH_PROVIDER_GITHUB_COPILOT),
        AUTH_PROVIDER_CODEX_OAUTH => Ok(AUTH_PROVIDER_CODEX_OAUTH),
        _ => Err(format!("Unsupported auth provider: {auth_provider}")),
    }
}

fn map_account(
    provider: &str,
    account: GitHubAccount,
    default_account_id: Option<&str>,
) -> ManagedAuthAccount {
    ManagedAuthAccount {
        is_default: default_account_id == Some(account.id.as_str()),
        id: account.id,
        provider: provider.to_string(),
        login: account.login,
        avatar_url: account.avatar_url,
        authenticated_at: account.authenticated_at,
        github_domain: account.github_domain,
    }
}

fn map_device_code_response(
    provider: &str,
    response: GitHubDeviceCodeResponse,
) -> ManagedAuthDeviceCodeResponse {
    ManagedAuthDeviceCodeResponse {
        provider: provider.to_string(),
        device_code: response.device_code,
        user_code: response.user_code,
        verification_uri: response.verification_uri,
        expires_in: response.expires_in,
        interval: response.interval,
    }
}

pub(crate) fn current_codex_official_provider(state: &AppState) -> Result<Provider, String> {
    let current_id = crate::settings::get_effective_current_provider(&state.db, &AppType::Codex)
        .map_err(|e| format!("读取当前 Codex provider 失败: {e}"))?
        .ok_or_else(|| "当前没有生效中的 Codex provider".to_string())?;

    let provider = state
        .db
        .get_provider_by_id(&current_id, AppType::Codex.as_str())
        .map_err(|e| format!("读取当前 Codex provider 详情失败: {e}"))?
        .ok_or_else(|| format!("当前 Codex provider 不存在: {current_id}"))?;

    if provider.category.as_deref() != Some("official") {
        return Err(
            "当前生效的 Codex provider 不是 OpenAI Official，请先切到官方 provider 再切账号"
                .to_string(),
        );
    }

    Ok(provider)
}

fn set_codex_account_binding(provider: &mut Provider, account_id: Option<String>) {
    let mut meta = provider.meta.clone().unwrap_or_default();
    meta.auth_binding = Some(AuthBinding {
        source: AuthBindingSource::ManagedAccount,
        auth_provider: Some(AUTH_PROVIDER_CODEX_OAUTH.to_string()),
        account_id,
    });
    provider.meta = Some(meta);
}

fn classify_codex_switch_precheck_error(err: CodexOAuthError) -> String {
    match err {
        CodexOAuthError::RefreshTokenInvalid => {
            "CODEX_OAUTH_REAUTH_REQUIRED: 当前账号的登录态已失效，请重新登录 ChatGPT 账号"
                .to_string()
        }
        CodexOAuthError::ParseError(message)
            if message.contains("缺少 id_token") || message.contains("持久化 auth.json 快照") =>
        {
            "CODEX_OAUTH_IMPORT_CURRENT_REQUIRED: 当前账号缺少完整桌面登录态，请先导入当前 Codex 登录或重新登录 ChatGPT 账号"
                .to_string()
        }
        CodexOAuthError::AccountNotFound(account_id) => {
            format!("CODEX_OAUTH_ACCOUNT_MISSING: 找不到要切换的 ChatGPT 账号：{account_id}")
        }
        CodexOAuthError::NetworkError(message)
        | CodexOAuthError::TokenFetchFailed(message)
        | CodexOAuthError::IoError(message) => format!(
            "CODEX_OAUTH_TEMPORARY_FAILURE: 当前无法刷新 ChatGPT 登录态，请稍后重试：{message}"
        ),
        other => format!("CODEX_OAUTH_SWITCH_PRECHECK_FAILED: 无法切换账号：{other}"),
    }
}

fn resolve_codex_switched_account(
    status: crate::proxy::providers::codex_oauth_auth::CodexOAuthStatus,
    preferred_account_id: Option<&str>,
) -> Result<ManagedAuthAccount, String> {
    let default_account_id = status.default_account_id.clone();
    let effective_account_id = preferred_account_id
        .map(ToOwned::to_owned)
        .or(default_account_id.clone())
        .ok_or_else(|| "当前没有可用的 ChatGPT 账号".to_string())?;

    let account = status
        .accounts
        .into_iter()
        .find(|account| account.id == effective_account_id)
        .ok_or_else(|| format!("找不到要切换的 ChatGPT 账号: {effective_account_id}"))?;

    Ok(map_account(
        AUTH_PROVIDER_CODEX_OAUTH,
        account,
        default_account_id.as_deref(),
    ))
}

async fn resolve_codex_switch_target(
    manager: &crate::proxy::providers::codex_oauth_auth::CodexOAuthManager,
    preferred_account_id: Option<&str>,
) -> Result<ManagedAuthAccount, String> {
    manager
        .build_native_auth(preferred_account_id)
        .await
        .map_err(classify_codex_switch_precheck_error)?;

    let status = manager.get_status().await;
    resolve_codex_switched_account(status, preferred_account_id)
}

pub(crate) async fn switch_codex_desktop_account(
    state: &AppState,
    account_id: Option<String>,
) -> Result<ManagedAuthAccount, String> {
    let manager = crate::proxy::providers::codex_oauth_auth::CodexOAuthManager::new(
        crate::config::get_app_config_dir(),
    );
    if let Err(err) = manager.sync_current_native_auth_if_tracked().await {
        log::debug!("[CodexOAuth] 切换前预同步当前 Codex 登录态失败，继续切换: {err}");
    }
    let switched_account = resolve_codex_switch_target(&manager, account_id.as_deref()).await?;

    let original_provider = current_codex_official_provider(state)?;
    let mut updated_provider = original_provider.clone();
    set_codex_account_binding(&mut updated_provider, account_id);

    crate::services::codex_desktop::switch_desktop_to_provider_with_runtime_auth_sync(
        state,
        &updated_provider,
        Some(&original_provider),
        false,
    )
    .await
    .map_err(|e| format!("切换当前 Codex 官方账号失败: {e}"))?;

    Ok(switched_account)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_start_login(
    auth_provider: String,
    github_domain: Option<String>,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<ManagedAuthDeviceCodeResponse, String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.read().await;
            let response = auth_manager
                .start_device_flow(github_domain.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            Ok(map_device_code_response(auth_provider, response))
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.read().await;
            let response = auth_manager
                .start_device_flow()
                .await
                .map_err(|e| e.to_string())?;
            Ok(map_device_code_response(auth_provider, response))
        }
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_poll_for_account(
    auth_provider: String,
    device_code: String,
    github_domain: Option<String>,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<Option<ManagedAuthAccount>, String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.write().await;
            match auth_manager
                .poll_for_token(&device_code, github_domain.as_deref())
                .await
            {
                Ok(account) => {
                    let default_account_id = auth_manager.get_status().await.default_account_id;
                    Ok(account.map(|account| {
                        map_account(auth_provider, account, default_account_id.as_deref())
                    }))
                }
                Err(CopilotAuthError::AuthorizationPending) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.write().await;
            match auth_manager.poll_for_token(&device_code).await {
                Ok(account) => {
                    let default_account_id = auth_manager.get_status().await.default_account_id;
                    Ok(account.map(|account| {
                        map_account(auth_provider, account, default_account_id.as_deref())
                    }))
                }
                Err(CodexOAuthError::AuthorizationPending) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        }
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_list_accounts(
    auth_provider: String,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<Vec<ManagedAuthAccount>, String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.read().await;
            let status = auth_manager.get_status().await;
            let default_account_id = status.default_account_id.clone();
            Ok(status
                .accounts
                .into_iter()
                .map(|account| map_account(auth_provider, account, default_account_id.as_deref()))
                .collect())
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.read().await;
            let status = auth_manager.get_status().await;
            let default_account_id = status.default_account_id.clone();
            Ok(status
                .accounts
                .into_iter()
                .map(|account| map_account(auth_provider, account, default_account_id.as_deref()))
                .collect())
        }
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_get_status(
    auth_provider: String,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<ManagedAuthStatus, String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.read().await;
            let status = auth_manager.get_status().await;
            let default_account_id = status.default_account_id.clone();
            Ok(ManagedAuthStatus {
                provider: auth_provider.to_string(),
                authenticated: status.authenticated,
                default_account_id: default_account_id.clone(),
                current_account_id: None,
                current_account_login: None,
                migration_error: status.migration_error,
                accounts: status
                    .accounts
                    .into_iter()
                    .map(|account| {
                        map_account(auth_provider, account, default_account_id.as_deref())
                    })
                    .collect(),
            })
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.read().await;
            if let Err(err) = auth_manager.sync_current_native_auth_if_tracked().await {
                log::debug!("[CodexOAuth] 跳过 auth status 自动同步: {err}");
            }
            let status = auth_manager.get_status().await;
            let default_account_id = status.default_account_id.clone();
            let current_account_id =
                crate::services::codex_desktop::read_current_codex_auth_account_id()
                    .ok()
                    .flatten();
            let current_account_login = current_account_id.as_ref().and_then(|account_id| {
                status
                    .accounts
                    .iter()
                    .find(|account| account.id == *account_id)
                    .map(|account| account.login.clone())
            });
            Ok(ManagedAuthStatus {
                provider: auth_provider.to_string(),
                authenticated: status.authenticated,
                default_account_id: default_account_id.clone(),
                current_account_id,
                current_account_login,
                migration_error: None,
                accounts: status
                    .accounts
                    .into_iter()
                    .map(|account| {
                        map_account(auth_provider, account, default_account_id.as_deref())
                    })
                    .collect(),
            })
        }
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_import_current(
    auth_provider: String,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<ManagedAuthAccount, String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_CODEX_OAUTH => {
            let (account, default_account_id) = {
                let auth_manager = codex_state.0.write().await;
                let account = auth_manager
                    .import_current_native_auth()
                    .await
                    .map_err(|e| e.to_string())?;
                let default_account_id = auth_manager.get_status().await.default_account_id;
                (account, default_account_id)
            };

            Ok(map_account(
                auth_provider,
                account,
                default_account_id.as_deref(),
            ))
        }
        AUTH_PROVIDER_GITHUB_COPILOT => Err("当前登录导入仅支持 Codex OAuth".to_string()),
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_switch_current_account(
    auth_provider: String,
    account_id: Option<String>,
    app_state: State<'_, AppState>,
) -> Result<ManagedAuthAccount, String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_CODEX_OAUTH => {
            switch_codex_desktop_account(app_state.inner(), account_id).await
        }
        AUTH_PROVIDER_GITHUB_COPILOT => Err("当前仅支持切换 Codex 官方账号".to_string()),
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_remove_account(
    auth_provider: String,
    account_id: String,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.write().await;
            auth_manager
                .remove_account(&account_id)
                .await
                .map_err(|e| e.to_string())
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.write().await;
            auth_manager
                .remove_account(&account_id)
                .await
                .map_err(|e| e.to_string())
        }
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_set_default_account(
    auth_provider: String,
    account_id: String,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.write().await;
            auth_manager
                .set_default_account(&account_id)
                .await
                .map_err(|e| e.to_string())
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.write().await;
            auth_manager
                .set_default_account(&account_id)
                .await
                .map_err(|e| e.to_string())
        }
        _ => unreachable!(),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_logout(
    auth_provider: String,
    copilot_state: State<'_, CopilotAuthState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    let auth_provider = ensure_auth_provider(&auth_provider)?;
    match auth_provider {
        AUTH_PROVIDER_GITHUB_COPILOT => {
            let auth_manager = copilot_state.0.write().await;
            auth_manager.clear_auth().await.map_err(|e| e.to_string())
        }
        AUTH_PROVIDER_CODEX_OAUTH => {
            let auth_manager = codex_state.0.write().await;
            auth_manager.clear_auth().await.map_err(|e| e.to_string())
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::provider::ProviderMeta;
    use serde_json::json;
    use std::sync::Arc;

    fn make_state_with_current_codex(provider: Provider) -> AppState {
        let db = Arc::new(Database::memory().expect("create in-memory db"));
        db.save_provider(AppType::Codex.as_str(), &provider)
            .expect("save provider");
        db.set_current_provider(AppType::Codex.as_str(), &provider.id)
            .expect("set current provider");
        AppState::new(db)
    }

    #[test]
    fn current_codex_official_provider_rejects_non_official_current_provider() {
        let mut provider = Provider::with_id(
            "codex-custom".to_string(),
            "Custom".to_string(),
            json!({ "auth": {}, "config": "" }),
            None,
        );
        provider.category = Some("third_party".to_string());

        let state = make_state_with_current_codex(provider);
        let err = current_codex_official_provider(&state).expect_err("should reject");
        assert!(err.contains("不是 OpenAI Official"));
    }

    #[test]
    fn set_codex_account_binding_uses_managed_account_binding() {
        let mut provider = Provider::with_id(
            "codex-official".to_string(),
            "OpenAI Official".to_string(),
            json!({ "auth": {}, "config": "" }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });

        set_codex_account_binding(&mut provider, Some("acc-123".to_string()));

        let binding = provider
            .meta
            .and_then(|meta| meta.auth_binding)
            .expect("auth binding should be set");
        assert_eq!(binding.source, AuthBindingSource::ManagedAccount);
        assert_eq!(
            binding.auth_provider.as_deref(),
            Some(AUTH_PROVIDER_CODEX_OAUTH)
        );
        assert_eq!(binding.account_id.as_deref(), Some("acc-123"));
    }
}
