//! Codex OAuth Tauri Commands
//!
//! 提供 OpenAI ChatGPT Plus/Pro OAuth 认证相关的 Tauri 命令。
//!
//! 大部分认证命令通过通用 `auth_*` 命令（参见 `commands::auth`）暴露给前端，
//! 此处定义 State wrapper 以及 Codex OAuth 专属的订阅额度查询命令。

use super::auth::switch_codex_desktop_account;
use crate::proxy::providers::codex_oauth_auth::{CodexOAuthError, CodexOAuthManager};
use crate::proxy::providers::copilot_auth::GitHubAccount;
use crate::services::codex_desktop::read_current_codex_auth_account_id;
use crate::services::subscription::{query_codex_quota, CredentialStatus, SubscriptionQuota};
use crate::store::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

/// Codex OAuth 认证状态
pub struct CodexOAuthState(pub Arc<RwLock<CodexOAuthManager>>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRotationCheckedAccount {
    pub account_id: String,
    pub account_login: String,
    pub available: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRotationResult {
    pub switched: bool,
    pub reason: String,
    pub from_account_id: Option<String>,
    pub to_account_id: Option<String>,
    pub to_account_login: Option<String>,
    pub checked_accounts: Vec<CodexRotationCheckedAccount>,
}

fn quota_allows_rotation(quota: &SubscriptionQuota) -> bool {
    if !quota.success || !matches!(quota.credential_status, CredentialStatus::Valid) {
        return false;
    }

    quota.tiers.is_empty() || quota.tiers.iter().any(|tier| tier.utilization < 100.0)
}

fn quota_result_from_codex_oauth_error(err: CodexOAuthError) -> SubscriptionQuota {
    match err {
        CodexOAuthError::RefreshTokenInvalid => SubscriptionQuota::error(
            "codex_oauth",
            CredentialStatus::Expired,
            "Codex OAuth sign-in is invalid. Please re-authenticate in cc-switch.".to_string(),
        ),
        CodexOAuthError::AccountNotFound(_) => SubscriptionQuota::not_found("codex_oauth"),
        CodexOAuthError::ParseError(message) => {
            SubscriptionQuota::error("codex_oauth", CredentialStatus::Expired, message)
        }
        CodexOAuthError::NetworkError(message)
        | CodexOAuthError::TokenFetchFailed(message)
        | CodexOAuthError::IoError(message) => SubscriptionQuota::error(
            "codex_oauth",
            CredentialStatus::Valid,
            format!("Unable to refresh Codex OAuth token right now: {message}"),
        ),
        other => {
            SubscriptionQuota::error("codex_oauth", CredentialStatus::Expired, other.to_string())
        }
    }
}

fn quota_rejection_reason(quota: &SubscriptionQuota) -> String {
    if !matches!(quota.credential_status, CredentialStatus::Valid) {
        return quota
            .credential_message
            .clone()
            .unwrap_or_else(|| "账号凭据不可用".to_string());
    }

    if !quota.success {
        return quota
            .error
            .clone()
            .or_else(|| quota.credential_message.clone())
            .unwrap_or_else(|| "额度查询失败".to_string());
    }

    if quota.tiers.is_empty() {
        return "额度接口未返回窗口数据".to_string();
    }

    let summary = quota
        .tiers
        .iter()
        .map(|tier| format!("{}={:.1}%", tier.name, tier.utilization))
        .collect::<Vec<_>>()
        .join(", ");

    format!("账号额度已满或接近上限: {summary}")
}

fn ordered_accounts_after_current(
    accounts: &[GitHubAccount],
    current_account_id: Option<&str>,
) -> Vec<GitHubAccount> {
    if accounts.is_empty() {
        return Vec::new();
    }

    let Some(current_id) = current_account_id else {
        return accounts.to_vec();
    };

    let Some(current_index) = accounts.iter().position(|item| item.id == current_id) else {
        return accounts.to_vec();
    };

    accounts[current_index + 1..]
        .iter()
        .chain(accounts[..current_index].iter())
        .cloned()
        .collect()
}

async fn inspect_rotation_candidate(
    manager: &CodexOAuthManager,
    account: &GitHubAccount,
) -> CodexRotationCheckedAccount {
    let account_id = account.id.clone();
    let account_login = account.login.clone();

    if let Err(err) = manager.build_native_auth(Some(&account_id)).await {
        return CodexRotationCheckedAccount {
            account_id,
            account_login,
            available: false,
            reason: format!("该账号无法生成桌面原生登录态: {err}"),
        };
    }

    let token = match manager.get_readonly_token_for_account(&account_id).await {
        Ok(token) => token,
        Err(err) => {
            return CodexRotationCheckedAccount {
                account_id,
                account_login,
                available: false,
                reason: format!("无法获取该账号的有效 token: {err}"),
            };
        }
    };

    let quota = query_codex_quota(
        &token,
        Some(&account_id),
        "codex_oauth",
        "Codex OAuth access token expired or rejected. Please re-login via cc-switch.",
    )
    .await;

    let available = quota_allows_rotation(&quota);
    let reason = if available {
        if quota.tiers.is_empty() {
            "账号可用，且额度接口未返回窗口数据".to_string()
        } else {
            let summary = quota
                .tiers
                .iter()
                .map(|tier| format!("{}={:.1}%", tier.name, tier.utilization))
                .collect::<Vec<_>>()
                .join(", ");
            format!("账号可用: {summary}")
        }
    } else {
        quota_rejection_reason(&quota)
    };

    CodexRotationCheckedAccount {
        account_id,
        account_login,
        available,
        reason,
    }
}

/// 查询 Codex OAuth (ChatGPT Plus/Pro) 订阅额度
///
/// - `account_id` 未指定时回退到 `CodexOAuthManager` 的默认账号
/// - 没有任何账号时返回 `not_found`，前端 `SubscriptionQuotaView` 会静默不渲染
/// - 复用 `services::subscription::query_codex_quota`，因此 wham/usage 端点协议
///   与 Codex CLI 路径完全一致
#[tauri::command(rename_all = "camelCase")]
pub async fn get_codex_oauth_quota(
    account_id: Option<String>,
    state: State<'_, CodexOAuthState>,
) -> Result<SubscriptionQuota, String> {
    let manager = state.0.read().await;

    let resolved = match account_id {
        Some(id) => Some(id),
        None => manager.default_account_id().await,
    };
    let Some(id) = resolved else {
        return Ok(SubscriptionQuota::not_found("codex_oauth"));
    };

    let token = match manager.get_readonly_token_for_account(&id).await {
        Ok(t) => t,
        Err(err) => return Ok(quota_result_from_codex_oauth_error(err)),
    };

    Ok(query_codex_quota(
        &token,
        Some(&id),
        "codex_oauth",
        "Codex OAuth access token expired or rejected. Please re-login via cc-switch.",
    )
    .await)
}

/// 自动检查当前 Codex 账号是否可用；如不可用则按账号池顺序切到下一个可用账号。
///
/// 设计约束：
/// - 自动轮换基于账号池顺序（默认账号优先，其余按最近登录时间排序）
/// - 切换动作复用桌面 App 主链路：保留同一份桌面数据目录，只重写 ~/.codex/auth.json，再重启
/// - `force_next=true` 时跳过当前账号的可用性判断，直接尝试下一个账号
#[tauri::command(rename_all = "camelCase")]
pub async fn rotate_codex_account(
    force_next: Option<bool>,
    app_state: State<'_, AppState>,
    codex_state: State<'_, CodexOAuthState>,
) -> Result<CodexRotationResult, String> {
    let force_next = force_next.unwrap_or(false);
    let current_account_id = read_current_codex_auth_account_id().map_err(|e| e.to_string())?;
    let mut checked_accounts = Vec::new();

    {
        let manager = codex_state.0.read().await;
        let status = manager.get_status().await;
        let accounts = status.accounts;

        if accounts.is_empty() {
            return Ok(CodexRotationResult {
                switched: false,
                reason: "账号池为空，无法自动切换 Codex 账号".to_string(),
                from_account_id: current_account_id,
                to_account_id: None,
                to_account_login: None,
                checked_accounts,
            });
        }

        if !force_next {
            if let Some(current_id) = current_account_id.as_deref() {
                if let Some(current_account) =
                    accounts.iter().find(|account| account.id == current_id)
                {
                    let current_check = inspect_rotation_candidate(&manager, current_account).await;
                    if current_check.available {
                        checked_accounts.push(current_check.clone());
                        return Ok(CodexRotationResult {
                            switched: false,
                            reason: "当前账号仍可用，无需切换".to_string(),
                            from_account_id: current_account_id,
                            to_account_id: Some(current_check.account_id),
                            to_account_login: Some(current_check.account_login),
                            checked_accounts,
                        });
                    }
                    checked_accounts.push(current_check);
                } else {
                    checked_accounts.push(CodexRotationCheckedAccount {
                        account_id: current_id.to_string(),
                        account_login: format!("ChatGPT ({current_id})"),
                        available: false,
                        reason: "当前桌面账号不在 cc-switch 账号池中，将尝试切到下一个受管账号"
                            .to_string(),
                    });
                }
            }
        }

        let account_candidates =
            ordered_accounts_after_current(&accounts, current_account_id.as_deref());

        for account in account_candidates {
            if current_account_id.as_deref() == Some(account.id.as_str()) {
                continue;
            }

            let inspected = inspect_rotation_candidate(&manager, &account).await;
            let should_switch = inspected.available;
            let target_account_id = inspected.account_id.clone();
            let target_account_login = inspected.account_login.clone();
            checked_accounts.push(inspected);

            if should_switch {
                drop(manager);

                switch_codex_desktop_account(app_state.inner(), Some(target_account_id.clone()))
                    .await?;

                return Ok(CodexRotationResult {
                    switched: true,
                    reason: "已切换到下一个可用 Codex 账号".to_string(),
                    from_account_id: current_account_id,
                    to_account_id: Some(target_account_id),
                    to_account_login: Some(target_account_login),
                    checked_accounts,
                });
            }
        }
    }

    Ok(CodexRotationResult {
        switched: false,
        reason: "账号池中没有找到可用的 Codex 账号".to_string(),
        from_account_id: current_account_id,
        to_account_id: None,
        to_account_login: None,
        checked_accounts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_accounts_wrap_after_current_account() {
        let accounts = vec![
            GitHubAccount {
                id: "a1".to_string(),
                login: "a1@example.com".to_string(),
                avatar_url: None,
                authenticated_at: 3,
                github_domain: "github.com".to_string(),
            },
            GitHubAccount {
                id: "a2".to_string(),
                login: "a2@example.com".to_string(),
                avatar_url: None,
                authenticated_at: 2,
                github_domain: "github.com".to_string(),
            },
            GitHubAccount {
                id: "a3".to_string(),
                login: "a3@example.com".to_string(),
                avatar_url: None,
                authenticated_at: 1,
                github_domain: "github.com".to_string(),
            },
        ];

        let ordered = ordered_accounts_after_current(&accounts, Some("a2"));
        let ids = ordered.into_iter().map(|item| item.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["a3".to_string(), "a1".to_string()]);
    }

    #[test]
    fn quota_allows_when_any_window_has_remaining_capacity() {
        let quota = SubscriptionQuota {
            tool: "codex_oauth".to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: vec![
                crate::services::subscription::QuotaTier {
                    name: "five_hour".to_string(),
                    utilization: 100.0,
                    resets_at: None,
                },
                crate::services::subscription::QuotaTier {
                    name: "seven_day".to_string(),
                    utilization: 64.0,
                    resets_at: None,
                },
            ],
            extra_usage: None,
            error: None,
            queried_at: None,
        };

        assert!(quota_allows_rotation(&quota));
    }

    #[test]
    fn quota_rejects_when_all_windows_are_full() {
        let quota = SubscriptionQuota {
            tool: "codex_oauth".to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: vec![crate::services::subscription::QuotaTier {
                name: "five_hour".to_string(),
                utilization: 100.0,
                resets_at: None,
            }],
            extra_usage: None,
            error: None,
            queried_at: None,
        };

        assert!(!quota_allows_rotation(&quota));
    }
}
