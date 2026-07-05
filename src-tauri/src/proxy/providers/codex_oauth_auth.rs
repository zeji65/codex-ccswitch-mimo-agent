//! Codex OAuth Authentication Module
//!
//! 实现 OpenAI ChatGPT Plus/Pro 订阅的 OAuth Device Code 流程。
//! 支持多账号管理，每个 Provider 可关联不同的 ChatGPT 账号。
//!
//! ## 认证流程
//! 1. 启动 Device Code 流程，获取 device_auth_id 和 user_code
//! 2. 用户在浏览器中完成 ChatGPT 授权
//! 3. 轮询获取 authorization_code 和 code_verifier（注意：verifier 由服务端返回）
//! 4. 使用 code + verifier 换取 access_token + refresh_token + id_token
//! 5. 自动刷新 access_token（到期前 60 秒）
//!
//! ## 多账号支持
//! - 每个 ChatGPT 账号独立存储 refresh_token
//! - Provider 通过 meta.authBinding 关联账号（auth_provider = "codex_oauth"）
//! - 通过 JWT id_token 提取 chatgpt_account_id 作为账号唯一标识

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use super::copilot_auth::{GitHubAccount, GitHubDeviceCodeResponse};

/// OpenAI OAuth 客户端 ID（OpenCode 使用，与官方 Codex CLI 相同）
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Device Code 启动 URL
const DEVICE_AUTH_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";

/// Device Code 轮询 URL
const DEVICE_AUTH_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";

/// OAuth Token URL（用于 code 换 token 和 refresh token）
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Device Code 验证 URL（向用户展示）
const DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";

/// Device Code 流程的 redirect_uri（OpenAI 服务端约定）
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

/// Token 刷新提前量（毫秒）
const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;

/// Device Code 默认有效时长（秒），OpenAI 文档约定 15 分钟
const DEVICE_CODE_DEFAULT_EXPIRES_IN: u64 = 900;

/// 轮询间隔安全余量（秒）
const POLLING_SAFETY_MARGIN_SECS: u64 = 3;

/// User-Agent
const CODEX_USER_AGENT: &str = "cc-switch-codex-oauth";

/// Codex OAuth 错误
#[derive(Debug, thiserror::Error)]
pub enum CodexOAuthError {
    #[error("等待用户授权中")]
    AuthorizationPending,

    #[error("用户拒绝授权")]
    AccessDenied,

    #[error("Device Code 已过期")]
    ExpiredToken,

    #[error("OAuth Token 获取失败: {0}")]
    TokenFetchFailed(String),

    #[error("Refresh Token 失效或已过期")]
    RefreshTokenInvalid,

    #[error("网络错误: {0}")]
    NetworkError(String),

    #[error("解析错误: {0}")]
    ParseError(String),

    #[error("IO 错误: {0}")]
    IoError(String),

    #[error("账号不存在: {0}")]
    AccountNotFound(String),

    #[error("账号已存在: {0}")]
    AccountAlreadyExists(String),
}

impl From<reqwest::Error> for CodexOAuthError {
    fn from(err: reqwest::Error) -> Self {
        CodexOAuthError::NetworkError(err.to_string())
    }
}

impl From<std::io::Error> for CodexOAuthError {
    fn from(err: std::io::Error) -> Self {
        CodexOAuthError::IoError(err.to_string())
    }
}

/// OpenAI Device Code 响应
#[derive(Debug, Clone, Deserialize)]
struct DeviceCodeResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: Option<serde_json::Value>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// OpenAI Device Code 轮询响应（成功）
#[derive(Debug, Clone, Deserialize)]
struct DevicePollSuccess {
    authorization_code: String,
    code_verifier: String,
}

/// OAuth Token 响应
#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Codex 原生 auth.json 结构（用于切换 Codex Desktop 登录态）
#[derive(Debug, Clone, Deserialize)]
struct NativeCodexAuth {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(default)]
    tokens: Option<NativeCodexTokens>,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeCodexTokens {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Clone)]
struct NativeCodexAccountSnapshot {
    account_id: String,
    email: Option<String>,
    id_token: Option<String>,
    refresh_token: String,
    access_token: Option<String>,
    native_auth: Value,
}

/// 解析后的 JWT claims（仅关心 chatgpt_account_id 等字段）
#[derive(Debug, Clone, Default, Deserialize)]
struct IdTokenClaims {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    organizations: Vec<OrgClaim>,
    #[serde(default, rename = "https://api.openai.com/auth")]
    openai_auth: Option<OpenAiAuthClaim>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OrgClaim {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenAiAuthClaim {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}

/// 缓存的 access_token（含过期时间）
#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    /// 过期时间戳（毫秒）
    expires_at_ms: i64,
}

impl CachedAccessToken {
    fn is_expiring_soon(&self) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        self.expires_at_ms - now < TOKEN_REFRESH_BUFFER_MS
    }
}

/// 进行中的 Device Code 条目，带过期时间以便清理放弃的登录流程
#[derive(Debug, Clone)]
struct PendingDeviceCode {
    user_code: String,
    /// Unix 毫秒时间戳，超时后可清理
    expires_at_ms: i64,
}

/// 持久化的账号数据
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexAccountData {
    /// CC Switch 管理用账号 ID（同时作为 HashMap 的 key）。
    ///
    /// 普通 OAuth 登录时与 ChatGPT 上游 account_id 相同；外部账号池 JSON 可能出现
    /// 多个账号共用同一个上游 account_id，此时这里会保存一个可选择的唯一槽位 ID。
    pub account_id: String,
    /// ChatGPT/Codex 上游真实 account_id。缺省时等于 account_id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_account_id: Option<String>,
    /// 账号邮箱（如果可获取）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// 用户自定义显示名。存在时优先于 email 用于 UI 展示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 最近一次拿到的 id_token，用于生成 Codex 原生 auth.json
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// Refresh Token（持久化）
    pub refresh_token: String,
    /// 持久化的原生 auth.json 快照，供 Codex Desktop 切号直接复用
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_auth: Option<Value>,
    /// 认证时间戳（秒）
    pub authenticated_at: i64,
}

impl CodexAccountData {
    fn upstream_account_id(&self) -> &str {
        self.upstream_account_id
            .as_deref()
            .unwrap_or(&self.account_id)
    }
}

/// 公开的账号信息（返回给前端，复用 GitHubAccount 结构）
impl From<&CodexAccountData> for GitHubAccount {
    fn from(data: &CodexAccountData) -> Self {
        GitHubAccount {
            id: data.account_id.clone(),
            // 用自定义显示名或 email 作为展示名（若无则用 account_id）
            login: data
                .display_name
                .clone()
                .or_else(|| data.email.clone())
                .unwrap_or_else(|| format!("ChatGPT ({})", &data.account_id)),
            avatar_url: None,
            authenticated_at: data.authenticated_at,
            github_domain: "github.com".to_string(),
        }
    }
}

/// 持久化存储结构（v2，兼容 v1 缺省字段）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CodexOAuthStore {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: HashMap<String, CodexAccountData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ExternalCodexAccountJson {
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone)]
struct ExternalCodexAccountImport {
    upstream_account_id: String,
    refresh_token: String,
    id_token: Option<String>,
    email: Option<String>,
    display_name: Option<String>,
    native_auth: Option<Value>,
}

/// Codex OAuth 认证管理器（多账号）
pub struct CodexOAuthManager {
    accounts: Arc<RwLock<HashMap<String, CodexAccountData>>>,
    default_account_id: Arc<RwLock<Option<String>>>,
    /// 内存缓存的 access_token（不持久化）
    access_tokens: Arc<RwLock<HashMap<String, CachedAccessToken>>>,
    /// 每个账号的刷新锁
    refresh_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// 进行中的 Device Code 流程：device_auth_id -> {user_code, expires_at_ms}
    /// 过期条目会在 start_device_flow 时被清理，防止放弃的登录流程导致无界增长
    pending_device_codes: Arc<RwLock<HashMap<String, PendingDeviceCode>>>,
    storage_path: PathBuf,
}

impl CodexOAuthManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let storage_path = data_dir.join("codex_oauth_auth.json");

        let manager = Self {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            default_account_id: Arc::new(RwLock::new(None)),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
            refresh_locks: Arc::new(RwLock::new(HashMap::new())),
            pending_device_codes: Arc::new(RwLock::new(HashMap::new())),
            storage_path,
        };

        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[CodexOAuth] 加载存储失败: {e}");
        }

        manager
    }

    // ==================== 设备码流程 ====================

    /// 启动 Device Code 流程
    ///
    /// 返回 GitHubDeviceCodeResponse 复用现有前端结构，但字段含义对应 OpenAI 的字段：
    /// - device_code = device_auth_id
    /// - user_code = user_code
    /// - verification_uri = https://auth.openai.com/codex/device
    pub async fn start_device_flow(&self) -> Result<GitHubDeviceCodeResponse, CodexOAuthError> {
        log::info!("[CodexOAuth] 启动 Device Code 流程");

        let response = crate::proxy::http_client::get()
            .post(DEVICE_AUTH_USERCODE_URL)
            .header("Content-Type", "application/json")
            .header("User-Agent", CODEX_USER_AGENT)
            .json(&serde_json::json!({ "client_id": CODEX_CLIENT_ID }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::NetworkError(format!(
                "Device Code 请求失败: {status} - {text}"
            )));
        }

        let device: DeviceCodeResponse = response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        let interval = parse_interval(device.interval.as_ref());
        let expires_in = device.expires_in.unwrap_or(DEVICE_CODE_DEFAULT_EXPIRES_IN);
        let expires_at_ms = chrono::Utc::now().timestamp_millis() + (expires_in as i64) * 1000;

        // 记录 device_auth_id -> 用户码映射；同时清理所有已过期的条目，
        // 避免用户放弃登录流程导致 HashMap 无界增长
        {
            let mut pending = self.pending_device_codes.write().await;
            let now_ms = chrono::Utc::now().timestamp_millis();
            pending.retain(|_, entry| entry.expires_at_ms > now_ms);
            pending.insert(
                device.device_auth_id.clone(),
                PendingDeviceCode {
                    user_code: device.user_code.clone(),
                    expires_at_ms,
                },
            );
        }

        log::info!(
            "[CodexOAuth] 获取 Device Code 成功，user_code: {}",
            device.user_code
        );

        Ok(GitHubDeviceCodeResponse {
            device_code: device.device_auth_id,
            user_code: device.user_code,
            verification_uri: DEVICE_VERIFICATION_URL.to_string(),
            expires_in,
            interval,
        })
    }

    /// 轮询 Device Code 状态
    ///
    /// 接收 device_code（即 device_auth_id），返回 Some(account) 表示授权成功
    pub async fn poll_for_token(
        &self,
        device_code: &str,
    ) -> Result<Option<GitHubAccount>, CodexOAuthError> {
        let entry = {
            let pending = self.pending_device_codes.read().await;
            pending.get(device_code).cloned()
        };

        let entry = entry.ok_or_else(|| {
            CodexOAuthError::TokenFetchFailed(
                "未找到对应的 user_code，请重新启动登录流程".to_string(),
            )
        })?;

        if entry.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
            let mut pending = self.pending_device_codes.write().await;
            pending.remove(device_code);
            return Err(CodexOAuthError::ExpiredToken);
        }

        let user_code = entry.user_code;

        log::debug!("[CodexOAuth] 轮询 Device Code");

        let poll_response = crate::proxy::http_client::get()
            .post(DEVICE_AUTH_TOKEN_URL)
            .header("Content-Type", "application/json")
            .header("User-Agent", CODEX_USER_AGENT)
            .json(&serde_json::json!({
                "device_auth_id": device_code,
                "user_code": user_code,
            }))
            .send()
            .await?;

        let status = poll_response.status();

        // 403/404 表示用户未完成授权，继续轮询
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND {
            return Err(CodexOAuthError::AuthorizationPending);
        }

        if status == reqwest::StatusCode::GONE {
            return Err(CodexOAuthError::ExpiredToken);
        }

        if !status.is_success() {
            let text = poll_response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "{status} - {text}"
            )));
        }

        let success: DevicePollSuccess = poll_response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        log::info!("[CodexOAuth] 用户已授权，正在换取 OAuth Token");

        // 用 authorization_code + code_verifier 换 token
        let tokens = self
            .exchange_code_for_tokens(&success.authorization_code, &success.code_verifier)
            .await?;

        // 清理 pending device code
        {
            let mut pending = self.pending_device_codes.write().await;
            pending.remove(device_code);
        }

        let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
            CodexOAuthError::TokenFetchFailed("响应缺少 refresh_token".to_string())
        })?;

        let (account_id, email) = extract_identity_from_tokens(&tokens);
        let account_id = account_id.ok_or_else(|| {
            CodexOAuthError::ParseError("无法从 token 中提取 account_id".to_string())
        })?;

        // 缓存 access_token
        {
            let mut tokens_cache = self.access_tokens.write().await;
            tokens_cache.insert(
                account_id.clone(),
                CachedAccessToken {
                    token: tokens.access_token.clone(),
                    expires_at_ms: compute_expires_at_ms(tokens.expires_in),
                },
            );
        }

        let native_auth = build_native_auth_json(
            &account_id,
            &tokens.access_token,
            &refresh_token,
            tokens.id_token.as_deref(),
        );

        let account = self
            .add_account_internal(
                account_id,
                refresh_token,
                tokens.id_token,
                email,
                None,
                Some(native_auth),
            )
            .await?;

        Ok(Some(account))
    }

    /// 用 authorization_code + code_verifier 换取 tokens
    async fn exchange_code_for_tokens(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        let response = crate::proxy::http_client::get()
            .post(OAUTH_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", CODEX_USER_AGENT)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", DEVICE_REDIRECT_URI),
                ("client_id", CODEX_CLIENT_ID),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "Token 交换失败: {status} - {text}"
            )));
        }

        response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))
    }

    /// 用 refresh_token 刷新 access_token
    async fn refresh_with_token(
        &self,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        let response = crate::proxy::http_client::get()
            .post(OAUTH_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", CODEX_USER_AGENT)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CODEX_CLIENT_ID),
                ("scope", "openid profile email"),
            ])
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(CodexOAuthError::RefreshTokenInvalid);
        }

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "Refresh 失败: {status} - {text}"
            )));
        }

        response
            .json()
            .await
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))
    }

    // ==================== Token 获取（含自动刷新） ====================

    /// 获取指定账号的有效 access_token（必要时自动刷新）
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        // 先检查缓存
        {
            let tokens = self.access_tokens.read().await;
            if let Some(cached) = tokens.get(account_id) {
                if !cached.is_expiring_soon() {
                    return Ok(cached.token.clone());
                }
            }
        }

        log::info!("[CodexOAuth] 账号 {account_id} 的 access_token 需要刷新");

        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _guard = refresh_lock.lock().await;

        // double-check
        {
            let tokens = self.access_tokens.read().await;
            if let Some(cached) = tokens.get(account_id) {
                if !cached.is_expiring_soon() {
                    return Ok(cached.token.clone());
                }
            }
        }

        let (refresh_token, upstream_account_id) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
            (
                account.refresh_token.clone(),
                account.upstream_account_id().to_string(),
            )
        };

        // The refresh_token may be empty or already consumed by another client
        // (e.g. Codex Desktop running concurrently — OpenAI refresh_tokens are
        // single-use and rotate on every refresh). In that case, fall back to the
        // access_token persisted in the native_auth snapshot, which is often still
        // valid for hours even when the refresh_token is dead.
        let new_tokens = match self.refresh_with_token(&refresh_token).await {
            Ok(tokens) => tokens,
            Err(refresh_err) => {
                if let Some(token) = self.persisted_access_token_for_account(account_id).await {
                    log::warn!(
                        "[CodexOAuth] 账号 {account_id} 刷新失败（{refresh_err}），回退使用持久化 access_token"
                    );
                    // Cache the fallback token with a short TTL so we re-evaluate
                    // soon, but don't hammer the dead refresh endpoint every call.
                    let mut tokens = self.access_tokens.write().await;
                    tokens.insert(
                        account_id.to_string(),
                        CachedAccessToken {
                            token: token.clone(),
                            expires_at_ms: chrono::Utc::now().timestamp_millis() + 5 * 60 * 1000,
                        },
                    );
                    return Ok(token);
                }
                return Err(refresh_err);
            }
        };

        let mut should_save = false;
        {
            let mut accounts = self.accounts.write().await;
            if let Some(account) = accounts.get_mut(account_id) {
                if let Some(new_refresh) = new_tokens.refresh_token.clone() {
                    if new_refresh != account.refresh_token {
                        account.refresh_token = new_refresh;
                        should_save = true;
                    }
                }

                if let Some(id_token) = new_tokens.id_token.clone() {
                    if account.id_token.as_ref() != Some(&id_token) {
                        account.id_token = Some(id_token);
                        should_save = true;
                    }
                }

                let native_auth = build_native_auth_json(
                    &upstream_account_id,
                    &new_tokens.access_token,
                    &account.refresh_token,
                    account.id_token.as_deref(),
                );
                if account.native_auth.as_ref() != Some(&native_auth) {
                    account.native_auth = Some(native_auth);
                    should_save = true;
                }
            }
        }

        if should_save {
            self.save_to_disk().await?;
        }

        let access_token = new_tokens.access_token.clone();
        let expires_at_ms = compute_expires_at_ms(new_tokens.expires_in);

        {
            let mut tokens = self.access_tokens.write().await;
            tokens.insert(
                account_id.to_string(),
                CachedAccessToken {
                    token: access_token.clone(),
                    expires_at_ms,
                },
            );
        }

        Ok(access_token)
    }

    /// 获取默认账号的有效 token
    pub async fn get_valid_token(&self) -> Result<String, CodexOAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.get_valid_token_for_account(&id).await,
            None => Err(CodexOAuthError::AccountNotFound(
                "无可用的 ChatGPT 账号".to_string(),
            )),
        }
    }

    /// 获取默认账号 ID（热路径使用，避免克隆整个账号 HashMap）
    pub async fn default_account_id(&self) -> Option<String> {
        self.resolve_default_account_id().await
    }

    /// 获取某个管理槽位对应的上游 ChatGPT account_id。
    pub async fn upstream_account_id_for_account(&self, account_id: &str) -> Option<String> {
        let accounts = self.accounts.read().await;
        accounts
            .get(account_id)
            .map(|account| account.upstream_account_id().to_string())
    }

    /// 生成 Codex Desktop / CLI 可识别的原生 auth.json。
    ///
    /// 优先复用导入或登录时持久化的原生快照，避免猜测 Codex auth.json
    /// 字段；若当前 runtime 正好是目标账号，会先同步一次快照。
    pub async fn build_native_auth(
        &self,
        preferred_account_id: Option<&str>,
    ) -> Result<Value, CodexOAuthError> {
        let account_id = match preferred_account_id {
            Some(id) => id.to_string(),
            None => self.resolve_default_account_id().await.ok_or_else(|| {
                CodexOAuthError::AccountNotFound("无可用的 ChatGPT 账号".to_string())
            })?,
        };
        let upstream_account_id = self
            .upstream_account_id_for_existing_account(&account_id)
            .await?;

        if let Err(err) = self.sync_current_native_auth_for_account(&account_id).await {
            log::debug!(
                "[CodexOAuth] 跳过当前 runtime auth 切号同步（account={account_id}）: {err}"
            );
        }

        let snapshot = self
            .read_persisted_native_auth_snapshot(&account_id)
            .await?;
        if snapshot.account_id != upstream_account_id {
            return Err(CodexOAuthError::ParseError(format!(
                "该账号的持久化 auth.json 快照与账号 ID 不一致，无法直接切换（expected={upstream_account_id}, actual={})",
                snapshot.account_id
            )));
        }

        if snapshot.id_token.is_none() {
            return Err(CodexOAuthError::ParseError(
                "该账号的持久化 auth.json 快照缺少 id_token，无法生成 Codex 原生登录态。请先在 Codex 中登录该账号，再回到 CC Switch 重新导入".to_string(),
            ));
        }

        Ok(snapshot.native_auth)
    }

    /// 从当前 Codex 原生 `~/.codex/auth.json` 导入当前登录账号。
    pub async fn import_current_native_auth(&self) -> Result<GitHubAccount, CodexOAuthError> {
        let snapshot = read_current_native_auth_snapshot()?;
        self.upsert_native_account_snapshot(snapshot, true)
            .await?
            .ok_or_else(|| CodexOAuthError::ParseError("导入当前 Codex 登录失败".to_string()))
    }

    /// 从外部 JSON 导入 Codex OAuth 账号。
    ///
    /// 这个入口用于受控导入已有的 account-pool JSON，不触发网络刷新，
    /// 也不会修改已有默认账号。重复账号会显式失败，避免静默覆盖。
    pub async fn import_external_account_json(
        &self,
        content: &str,
        display_name: Option<&str>,
    ) -> Result<GitHubAccount, CodexOAuthError> {
        let import = parse_external_account_json(content, display_name)?;
        let account_id = self.managed_account_id_for_external_import(&import).await?;

        let upstream_account_id = if account_id == import.upstream_account_id {
            None
        } else {
            Some(import.upstream_account_id)
        };

        self.add_account_record(
            account_id,
            upstream_account_id,
            import.refresh_token,
            import.id_token,
            import.email,
            import.display_name,
            import.native_auth,
        )
        .await
    }

    /// 若当前运行中的 Codex 账号已在账号池中，则同步最新 auth.json 快照。
    ///
    /// 不会静默新增陌生账号，只回补已受管账号的 token / id_token / email。
    pub async fn sync_current_native_auth_if_tracked(
        &self,
    ) -> Result<Option<GitHubAccount>, CodexOAuthError> {
        let snapshot = read_current_native_auth_snapshot()?;
        self.upsert_native_account_snapshot(snapshot, false).await
    }

    // ==================== 多账号管理 ====================

    pub async fn list_accounts(&self) -> Vec<GitHubAccount> {
        let accounts = self.accounts.read().await.clone();
        let default_id = self.resolve_default_account_id().await;
        Self::sorted_accounts(&accounts, default_id.as_deref())
    }

    pub async fn remove_account(&self, account_id: &str) -> Result<(), CodexOAuthError> {
        log::info!("[CodexOAuth] 移除账号: {account_id}");

        {
            let mut accounts = self.accounts.write().await;
            if accounts.remove(account_id).is_none() {
                return Err(CodexOAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        {
            let mut tokens = self.access_tokens.write().await;
            tokens.remove(account_id);
        }
        {
            let mut locks = self.refresh_locks.write().await;
            locks.remove(account_id);
        }

        {
            let accounts = self.accounts.read().await;
            let mut default = self.default_account_id.write().await;
            if default.as_deref() == Some(account_id) {
                *default = Self::fallback_default_account_id(&accounts);
            }
        }

        self.save_to_disk().await?;
        Ok(())
    }

    pub async fn set_default_account(&self, account_id: &str) -> Result<(), CodexOAuthError> {
        {
            let accounts = self.accounts.read().await;
            if !accounts.contains_key(account_id) {
                return Err(CodexOAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        {
            let mut default = self.default_account_id.write().await;
            *default = Some(account_id.to_string());
        }

        self.save_to_disk().await?;
        Ok(())
    }

    pub async fn clear_auth(&self) -> Result<(), CodexOAuthError> {
        log::info!("[CodexOAuth] 清除所有认证");

        {
            let mut accounts = self.accounts.write().await;
            accounts.clear();
        }
        {
            let mut default = self.default_account_id.write().await;
            *default = None;
        }
        {
            let mut tokens = self.access_tokens.write().await;
            tokens.clear();
        }
        {
            let mut locks = self.refresh_locks.write().await;
            locks.clear();
        }
        {
            let mut pending = self.pending_device_codes.write().await;
            pending.clear();
        }

        if self.storage_path.exists() {
            std::fs::remove_file(&self.storage_path)?;
        }

        Ok(())
    }

    pub async fn is_authenticated(&self) -> bool {
        let accounts = self.accounts.read().await;
        !accounts.is_empty()
    }

    /// 获取认证状态摘要（与 Copilot 的格式保持一致，便于复用前端）
    pub async fn get_status(&self) -> CodexOAuthStatus {
        let accounts_map = self.accounts.read().await.clone();
        let default_id = self.resolve_default_account_id().await;
        let account_list = Self::sorted_accounts(&accounts_map, default_id.as_deref());
        let authenticated = !account_list.is_empty();
        let username = default_id
            .as_ref()
            .and_then(|id| accounts_map.get(id))
            .and_then(|a| a.display_name.clone().or_else(|| a.email.clone()))
            .or_else(|| account_list.first().map(|a| a.login.clone()));

        CodexOAuthStatus {
            accounts: account_list,
            default_account_id: default_id,
            authenticated,
            username,
        }
    }

    // ==================== 内部方法 ====================

    async fn add_account_internal(
        &self,
        account_id: String,
        refresh_token: String,
        id_token: Option<String>,
        email: Option<String>,
        display_name: Option<String>,
        native_auth: Option<Value>,
    ) -> Result<GitHubAccount, CodexOAuthError> {
        self.add_account_record(
            account_id,
            None,
            refresh_token,
            id_token,
            email,
            display_name,
            native_auth,
        )
        .await
    }

    async fn add_account_record(
        &self,
        account_id: String,
        upstream_account_id: Option<String>,
        refresh_token: String,
        id_token: Option<String>,
        email: Option<String>,
        display_name: Option<String>,
        native_auth: Option<Value>,
    ) -> Result<GitHubAccount, CodexOAuthError> {
        let now = chrono::Utc::now().timestamp();

        let data = CodexAccountData {
            account_id: account_id.clone(),
            upstream_account_id,
            email,
            display_name,
            id_token,
            refresh_token,
            native_auth,
            authenticated_at: now,
        };

        let account = GitHubAccount::from(&data);

        {
            let mut accounts = self.accounts.write().await;
            accounts.insert(account_id.clone(), data);
        }

        {
            let mut default = self.default_account_id.write().await;
            if default.is_none() {
                *default = Some(account_id);
            }
        }

        self.save_to_disk().await?;
        Ok(account)
    }

    async fn upstream_account_id_for_existing_account(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        let accounts = self.accounts.read().await;
        let account = accounts
            .get(account_id)
            .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
        Ok(account.upstream_account_id().to_string())
    }

    async fn managed_account_id_for_external_import(
        &self,
        import: &ExternalCodexAccountImport,
    ) -> Result<String, CodexOAuthError> {
        let accounts = self.accounts.read().await;
        let upstream_id = import.upstream_account_id.as_str();

        if !accounts.contains_key(upstream_id) {
            return Ok(upstream_id.to_string());
        }

        if let Some(email) = import.email.as_deref() {
            if let Some((existing_id, _)) = accounts.iter().find(|(_, account)| {
                account.upstream_account_id() == upstream_id
                    && account.email.as_deref() == Some(email)
            }) {
                return Err(CodexOAuthError::AccountAlreadyExists(existing_id.clone()));
            }
        } else {
            return Err(CodexOAuthError::AccountAlreadyExists(
                upstream_id.to_string(),
            ));
        }

        let suffix_source = import
            .email
            .as_deref()
            .or(import.display_name.as_deref())
            .ok_or_else(|| CodexOAuthError::AccountAlreadyExists(upstream_id.to_string()))?;
        let suffix = sanitize_external_account_suffix(suffix_source);
        let base = format!("{upstream_id}#{suffix}");
        let mut candidate = base.clone();
        let mut counter = 2;
        while accounts.contains_key(&candidate) {
            candidate = format!("{base}-{counter}");
            counter += 1;
        }

        Ok(candidate)
    }

    async fn sync_current_native_auth_for_account(
        &self,
        account_id: &str,
    ) -> Result<Option<GitHubAccount>, CodexOAuthError> {
        let snapshot = read_current_native_auth_snapshot()?;
        let upstream_account_id = self
            .upstream_account_id_for_existing_account(account_id)
            .await?;
        if snapshot.account_id != upstream_account_id {
            return Ok(None);
        }
        self.upsert_native_account_snapshot(snapshot, false).await
    }

    async fn upsert_native_account_snapshot(
        &self,
        snapshot: NativeCodexAccountSnapshot,
        allow_new_account: bool,
    ) -> Result<Option<GitHubAccount>, CodexOAuthError> {
        let NativeCodexAccountSnapshot {
            account_id,
            email,
            id_token,
            refresh_token,
            access_token,
            native_auth,
        } = snapshot;

        let mut should_save = false;

        let existing_account = {
            let mut accounts = self.accounts.write().await;
            if let Some(account) = accounts.get_mut(&account_id) {
                if account.refresh_token != refresh_token {
                    account.refresh_token = refresh_token.clone();
                    should_save = true;
                }

                if let Some(native_id_token) = id_token.as_ref() {
                    if account.id_token.as_ref() != Some(native_id_token) {
                        account.id_token = Some(native_id_token.clone());
                        should_save = true;
                    }
                }

                if let Some(native_email) = email.as_ref() {
                    if account.email.as_ref() != Some(native_email) {
                        account.email = Some(native_email.clone());
                        should_save = true;
                    }
                }

                if account.native_auth.as_ref() != Some(&native_auth) {
                    account.native_auth = Some(native_auth.clone());
                    should_save = true;
                }

                Some(GitHubAccount::from(&*account))
            } else {
                None
            }
        };

        let account = match existing_account {
            Some(account) => account,
            None => {
                if !allow_new_account {
                    return Ok(None);
                }

                self.add_account_internal(
                    account_id.clone(),
                    refresh_token,
                    id_token,
                    email,
                    None,
                    Some(native_auth),
                )
                .await?
            }
        };

        if should_save {
            self.save_to_disk().await?;
        }

        if let Some(access_token) = access_token {
            let mut tokens_cache = self.access_tokens.write().await;
            tokens_cache.insert(
                account_id,
                CachedAccessToken {
                    token: access_token,
                    // Runtime auth token is only a snapshot; force refresh on real use.
                    expires_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            );
        }

        Ok(Some(account))
    }

    fn fallback_default_account_id(accounts: &HashMap<String, CodexAccountData>) -> Option<String> {
        accounts
            .iter()
            .max_by(|(id_a, a), (id_b, b)| {
                a.authenticated_at
                    .cmp(&b.authenticated_at)
                    .then_with(|| id_b.cmp(id_a))
            })
            .map(|(id, _)| id.clone())
    }

    fn sorted_accounts(
        accounts: &HashMap<String, CodexAccountData>,
        default_account_id: Option<&str>,
    ) -> Vec<GitHubAccount> {
        let mut list: Vec<GitHubAccount> = accounts.values().map(GitHubAccount::from).collect();
        list.sort_by(|a, b| {
            let a_default = default_account_id == Some(a.id.as_str());
            let b_default = default_account_id == Some(b.id.as_str());
            b_default
                .cmp(&a_default)
                .then_with(|| b.authenticated_at.cmp(&a.authenticated_at))
                .then_with(|| a.login.cmp(&b.login))
        });
        list
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;

        if let Some(id) = stored {
            if accounts.contains_key(&id) {
                return Some(id);
            }
        }

        Self::fallback_default_account_id(&accounts)
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let locks = self.refresh_locks.read().await;
            if let Some(lock) = locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.refresh_locks.write().await;
        Arc::clone(
            locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn write_store_atomic(&self, content: &str) -> Result<(), CodexOAuthError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let parent = self
            .storage_path
            .parent()
            .ok_or_else(|| CodexOAuthError::IoError("无效的存储路径".to_string()))?;
        let file_name = self
            .storage_path
            .file_name()
            .ok_or_else(|| CodexOAuthError::IoError("无效的存储文件名".to_string()))?
            .to_string_lossy()
            .to_string();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = parent.join(format!("{file_name}.tmp.{ts}"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            fs::rename(&tmp_path, &self.storage_path)?;
            fs::set_permissions(&self.storage_path, fs::Permissions::from_mode(0o600))?;
        }

        #[cfg(windows)]
        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            if self.storage_path.exists() {
                let _ = fs::remove_file(&self.storage_path);
            }
            fs::rename(&tmp_path, &self.storage_path)?;
        }

        Ok(())
    }

    fn load_from_disk_sync(&self) -> Result<(), CodexOAuthError> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)?;
        let store: CodexOAuthStore = serde_json::from_str(&content)
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        if let Ok(mut accounts) = self.accounts.try_write() {
            *accounts = store.accounts;
            log::info!("[CodexOAuth] 从磁盘加载 {} 个账号", accounts.len());
        }
        if let Ok(mut default) = self.default_account_id.try_write() {
            *default = store.default_account_id;
            if default.is_none() {
                if let Ok(accounts) = self.accounts.try_read() {
                    *default = Self::fallback_default_account_id(&accounts);
                }
            }
        }

        Ok(())
    }

    async fn save_to_disk(&self) -> Result<(), CodexOAuthError> {
        let accounts = self.accounts.read().await.clone();
        let default = self.resolve_default_account_id().await;

        let store = CodexOAuthStore {
            version: 1,
            accounts,
            default_account_id: default,
        };

        let content = serde_json::to_string_pretty(&store)
            .map_err(|e| CodexOAuthError::ParseError(e.to_string()))?;

        self.write_store_atomic(&content)?;

        log::info!(
            "[CodexOAuth] 保存到磁盘成功（{} 个账号）",
            store.accounts.len()
        );

        Ok(())
    }

    /// Read the access_token persisted in the account's native_auth snapshot.
    /// Returns None if the account has no snapshot or no access_token in it.
    /// Used as a fallback when the refresh_token is dead (consumed by another
    /// client) but the cached access_token may still be accepted by the server.
    async fn persisted_access_token_for_account(&self, account_id: &str) -> Option<String> {
        let accounts = self.accounts.read().await;
        let account = accounts.get(account_id)?;
        let native_auth = account.native_auth.as_ref()?;
        native_auth
            .get("tokens")
            .and_then(|t| t.get("access_token"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    }

    async fn read_persisted_native_auth_snapshot(
        &self,
        account_id: &str,
    ) -> Result<NativeCodexAccountSnapshot, CodexOAuthError> {
        let stored_native_auth = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
            account.native_auth.clone()
        };

        let native_auth = stored_native_auth.ok_or_else(|| {
            CodexOAuthError::ParseError(
                "该账号缺少持久化 auth.json 快照，无法直接切换。请先导入当前 Codex 登录或重新登录"
                    .to_string(),
            )
        })?;

        parse_native_auth_snapshot(native_auth, "账号池中的持久化 auth.json 快照")
    }
}

/// Codex OAuth 状态摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexOAuthStatus {
    pub accounts: Vec<GitHubAccount>,
    pub default_account_id: Option<String>,
    pub authenticated: bool,
    pub username: Option<String>,
}

// ==================== 工具函数 ====================

/// 解析 OpenAI Device Code 响应中的 interval 字段
///
/// 服务端可能返回字符串或数字，需要兼容
fn parse_interval(value: Option<&serde_json::Value>) -> u64 {
    let raw = match value {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(5),
        Some(serde_json::Value::String(s)) => s.parse::<u64>().unwrap_or(5),
        _ => 5,
    };
    raw.max(1) + POLLING_SAFETY_MARGIN_SECS
}

/// 从 expires_in（秒）计算过期时间戳（毫秒）
fn compute_expires_at_ms(expires_in: Option<i64>) -> i64 {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let secs = expires_in.unwrap_or(3600);
    now_ms + secs * 1000
}

fn build_native_auth_json(
    account_id: &str,
    access_token: &str,
    refresh_token: &str,
    id_token: Option<&str>,
) -> Value {
    let mut tokens = serde_json::Map::new();
    if let Some(token) = id_token {
        tokens.insert("id_token".to_string(), Value::String(token.to_string()));
    }
    tokens.insert(
        "access_token".to_string(),
        Value::String(access_token.to_string()),
    );
    tokens.insert(
        "refresh_token".to_string(),
        Value::String(refresh_token.to_string()),
    );
    tokens.insert(
        "account_id".to_string(),
        Value::String(account_id.to_string()),
    );

    serde_json::json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": Value::Null,
        "tokens": Value::Object(tokens),
        "last_refresh": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
    })
}

fn normalize_owned(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_str(value: Option<&str>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn sanitize_external_account_suffix(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '-' | '+') {
                ch
            } else {
                '-'
            }
        })
        .collect();

    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "external".to_string()
    } else {
        trimmed.chars().take(96).collect()
    }
}

fn ensure_matching_account_id(
    expected: &str,
    actual: Option<String>,
    source: &str,
) -> Result<(), CodexOAuthError> {
    if let Some(actual) = actual {
        if actual != expected {
            return Err(CodexOAuthError::ParseError(format!(
                "{source} 中的 account_id 与 JSON account_id 不一致"
            )));
        }
    }
    Ok(())
}

fn parse_external_account_json(
    content: &str,
    display_name: Option<&str>,
) -> Result<ExternalCodexAccountImport, CodexOAuthError> {
    let raw: ExternalCodexAccountJson = serde_json::from_str(content)
        .map_err(|e| CodexOAuthError::ParseError(format!("解析导入 JSON 失败: {e}")))?;

    let account_id = normalize_owned(raw.account_id);
    let chatgpt_account_id = normalize_owned(raw.chatgpt_account_id);
    if let (Some(account_id), Some(chatgpt_account_id)) = (&account_id, &chatgpt_account_id) {
        if account_id != chatgpt_account_id {
            return Err(CodexOAuthError::ParseError(
                "account_id 与 chatgpt_account_id 不一致".to_string(),
            ));
        }
    }

    let access_token = normalize_owned(raw.access_token);
    let id_token = normalize_owned(raw.id_token);
    let token_account_id = id_token
        .as_deref()
        .and_then(extract_account_id_from_jwt)
        .or_else(|| {
            access_token
                .as_deref()
                .and_then(extract_account_id_from_jwt)
        });

    let account_id = account_id
        .or(chatgpt_account_id)
        .or_else(|| token_account_id.clone())
        .ok_or_else(|| CodexOAuthError::ParseError("导入 JSON 缺少 account_id".to_string()))?;

    ensure_matching_account_id(&account_id, token_account_id, "token")?;

    let refresh_token = normalize_owned(raw.refresh_token)
        .ok_or_else(|| CodexOAuthError::ParseError("导入 JSON 缺少 refresh_token".to_string()))?;

    let email = normalize_owned(raw.email);
    let display_name = normalize_str(display_name).or_else(|| normalize_owned(raw.name));

    let native_auth = access_token.as_deref().map(|token| {
        build_native_auth_json(&account_id, token, &refresh_token, id_token.as_deref())
    });

    Ok(ExternalCodexAccountImport {
        upstream_account_id: account_id,
        refresh_token,
        id_token,
        email,
        display_name,
        native_auth,
    })
}

fn parse_native_auth_snapshot(
    native_auth: Value,
    source: &str,
) -> Result<NativeCodexAccountSnapshot, CodexOAuthError> {
    let native: NativeCodexAuth = serde_json::from_value(native_auth.clone())
        .map_err(|e| CodexOAuthError::ParseError(format!("解析{source}失败: {e}")))?;

    if native.auth_mode.as_deref() != Some("chatgpt") {
        return Err(CodexOAuthError::ParseError(format!(
            "{source}不是 ChatGPT OAuth 登录态"
        )));
    }

    let tokens = native
        .tokens
        .ok_or_else(|| CodexOAuthError::ParseError(format!("{source}缺少 tokens 字段")))?;

    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or_else(|| CodexOAuthError::ParseError(format!("{source}缺少 refresh_token")))?;

    let account_id = tokens
        .account_id
        .clone()
        .or_else(|| {
            tokens
                .id_token
                .as_deref()
                .and_then(extract_account_id_from_jwt)
        })
        .or_else(|| {
            tokens
                .access_token
                .as_deref()
                .and_then(extract_account_id_from_jwt)
        })
        .ok_or_else(|| CodexOAuthError::ParseError(format!("无法从{source}中解析 account_id")))?;

    let email = tokens
        .id_token
        .as_deref()
        .and_then(parse_jwt_claims)
        .and_then(|claims| claims.email)
        .or_else(|| {
            tokens
                .access_token
                .as_deref()
                .and_then(parse_jwt_claims)
                .and_then(|claims| claims.email)
        });

    Ok(NativeCodexAccountSnapshot {
        account_id,
        email,
        id_token: tokens.id_token,
        refresh_token,
        access_token: tokens.access_token,
        native_auth,
    })
}

fn read_current_native_auth_snapshot() -> Result<NativeCodexAccountSnapshot, CodexOAuthError> {
    let auth_path = crate::codex_config::get_codex_auth_path();
    let content = fs::read_to_string(&auth_path)?;
    let native_auth: Value = serde_json::from_str(&content)
        .map_err(|e| CodexOAuthError::ParseError(format!("解析 Codex auth.json 失败: {e}")))?;

    parse_native_auth_snapshot(native_auth, "当前 Codex auth.json")
}

fn extract_account_id_from_jwt(token: &str) -> Option<String> {
    let claims = parse_jwt_claims(token)?;
    claims
        .chatgpt_account_id
        .or_else(|| {
            claims
                .openai_auth
                .as_ref()
                .and_then(|a| a.chatgpt_account_id.clone())
        })
        .or_else(|| claims.organizations.first().and_then(|o| o.id.clone()))
}

/// 解析 JWT 中的 claims
fn parse_jwt_claims(token: &str) -> Option<IdTokenClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&decoded).ok()
}

/// 从 token 响应中提取 (account_id, email)
fn extract_identity_from_tokens(tokens: &OAuthTokenResponse) -> (Option<String>, Option<String>) {
    let mut account_id: Option<String> = None;
    let mut email: Option<String> = None;

    if let Some(id_token) = tokens.id_token.as_deref() {
        if let Some(claims) = parse_jwt_claims(id_token) {
            account_id = claims
                .chatgpt_account_id
                .clone()
                .or_else(|| {
                    claims
                        .openai_auth
                        .as_ref()
                        .and_then(|a| a.chatgpt_account_id.clone())
                })
                .or_else(|| claims.organizations.first().and_then(|o| o.id.clone()));
            email = claims.email.clone();
        }
    }

    if account_id.is_none() {
        if let Some(claims) = parse_jwt_claims(&tokens.access_token) {
            account_id = claims
                .chatgpt_account_id
                .clone()
                .or_else(|| {
                    claims
                        .openai_auth
                        .as_ref()
                        .and_then(|a| a.chatgpt_account_id.clone())
                })
                .or_else(|| claims.organizations.first().and_then(|o| o.id.clone()));
            if email.is_none() {
                email = claims.email.clone();
            }
        }
    }

    (account_id, email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_interval_number() {
        let v = serde_json::Value::Number(serde_json::Number::from(5));
        assert_eq!(parse_interval(Some(&v)), 5 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_parse_interval_string() {
        let v = serde_json::Value::String("10".to_string());
        assert_eq!(parse_interval(Some(&v)), 10 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_parse_interval_default() {
        assert_eq!(parse_interval(None), 5 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_parse_interval_min() {
        let v = serde_json::Value::Number(serde_json::Number::from(0));
        // 0 应被提升到 1
        assert_eq!(parse_interval(Some(&v)), 1 + POLLING_SAFETY_MARGIN_SECS);
    }

    #[test]
    fn test_compute_expires_at_ms() {
        let result = compute_expires_at_ms(Some(3600));
        let now = chrono::Utc::now().timestamp_millis();
        // 应在未来约 3600 秒处（允许少量误差）
        assert!(result > now + 3500 * 1000);
        assert!(result < now + 3700 * 1000);
    }

    #[test]
    fn test_compute_expires_at_ms_default() {
        let result = compute_expires_at_ms(None);
        let now = chrono::Utc::now().timestamp_millis();
        assert!(result > now);
    }

    #[test]
    fn test_cached_token_expiring_soon() {
        let now = chrono::Utc::now().timestamp_millis();
        // 30 秒后过期 - 在缓冲期内
        let expiring = CachedAccessToken {
            token: "t".to_string(),
            expires_at_ms: now + 30_000,
        };
        assert!(expiring.is_expiring_soon());

        // 1 小时后过期 - 不在缓冲期内
        let valid = CachedAccessToken {
            token: "t".to_string(),
            expires_at_ms: now + 3_600_000,
        };
        assert!(!valid.is_expiring_soon());
    }

    #[test]
    fn test_parse_jwt_claims_invalid() {
        assert!(parse_jwt_claims("not-a-jwt").is_none());
        assert!(parse_jwt_claims("only.two").is_none());
    }

    #[test]
    fn test_parse_jwt_claims_valid() {
        // Header: {"alg":"none"}
        // Payload: {"chatgpt_account_id":"acc-123","email":"test@example.com"}
        // Signature: empty
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD
            .encode(b"{\"chatgpt_account_id\":\"acc-123\",\"email\":\"test@example.com\"}");
        let jwt = format!("{header}.{payload}.");
        let claims = parse_jwt_claims(&jwt).unwrap();
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("acc-123"));
        assert_eq!(claims.email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn test_parse_jwt_claims_organizations_fallback() {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD.encode(b"{\"organizations\":[{\"id\":\"org-456\"}]}");
        let jwt = format!("{header}.{payload}.");
        let claims = parse_jwt_claims(&jwt).unwrap();
        assert_eq!(
            claims
                .organizations
                .first()
                .and_then(|o| o.id.clone())
                .as_deref(),
            Some("org-456")
        );
    }

    #[test]
    fn test_build_native_auth_json_shape() {
        let auth = build_native_auth_json("acc-123", "at", "rt", Some("id"));

        assert_eq!(auth["auth_mode"], "chatgpt");
        assert!(auth["OPENAI_API_KEY"].is_null());
        assert_eq!(auth["tokens"]["account_id"], "acc-123");
        assert_eq!(auth["tokens"]["access_token"], "at");
        assert_eq!(auth["tokens"]["refresh_token"], "rt");
        assert_eq!(auth["tokens"]["id_token"], "id");
        assert!(auth["last_refresh"].as_str().is_some());
    }

    #[test]
    fn test_parse_native_auth_snapshot_requires_chatgpt_auth() {
        let err = parse_native_auth_snapshot(
            serde_json::json!({
                "auth_mode": "apikey",
                "tokens": {
                    "account_id": "acc-123",
                    "refresh_token": "rt"
                }
            }),
            "测试 auth.json",
        )
        .expect_err("non-chatgpt auth should be rejected");

        assert!(err.to_string().contains("不是 ChatGPT OAuth 登录态"));
    }

    #[tokio::test]
    async fn test_manager_initial_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        assert!(!manager.is_authenticated().await);
        assert!(manager.list_accounts().await.is_empty());
    }

    #[tokio::test]
    async fn test_manager_save_and_load() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_path_buf();

        // Manually inject an account through internal methods
        {
            let manager = CodexOAuthManager::new(path.clone());
            manager
                .add_account_internal(
                    "acc-123".to_string(),
                    "rt-secret".to_string(),
                    None,
                    Some("user@example.com".to_string()),
                    None,
                    None,
                )
                .await
                .unwrap();
        }

        // New manager should load from disk
        let manager2 = CodexOAuthManager::new(path);
        let accounts = manager2.list_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "acc-123");
    }

    #[tokio::test]
    async fn test_import_external_account_json_adds_account_without_changing_default() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        manager
            .add_account_internal(
                "default-acc".to_string(),
                "rt-default".to_string(),
                None,
                Some("default@example.com".to_string()),
                None,
                None,
            )
            .await
            .unwrap();

        let account = manager
            .import_external_account_json(
                r#"{
                    "account_id": "external-acc",
                    "chatgpt_account_id": "external-acc",
                    "access_token": "access-token",
                    "refresh_token": "refresh-token",
                    "email": "raw@example.com",
                    "name": "Raw Name"
                }"#,
                Some("企业号"),
            )
            .await
            .unwrap();

        assert_eq!(account.id, "external-acc");
        assert_eq!(account.login, "企业号");

        let status = manager.get_status().await;
        assert_eq!(status.accounts.len(), 2);
        assert_eq!(status.default_account_id.as_deref(), Some("default-acc"));
        assert!(status
            .accounts
            .iter()
            .any(|account| account.id == "external-acc" && account.login == "企业号"));
    }

    #[tokio::test]
    async fn test_import_external_account_json_rejects_duplicate_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        manager
            .add_account_internal(
                "external-acc".to_string(),
                "original-refresh".to_string(),
                None,
                Some("Original".to_string()),
                None,
                None,
            )
            .await
            .unwrap();

        let err = manager
            .import_external_account_json(
                r#"{
                    "account_id": "external-acc",
                    "access_token": "new-access",
                    "refresh_token": "new-refresh"
                }"#,
                Some("企业号"),
            )
            .await
            .expect_err("duplicate import should fail");

        assert!(err.to_string().contains("已存在"));

        let accounts = manager.list_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].login, "Original");
    }

    #[tokio::test]
    async fn test_import_external_account_json_allows_same_upstream_with_different_email() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        manager
            .add_account_internal(
                "shared-upstream".to_string(),
                "original-refresh".to_string(),
                None,
                Some("old@example.com".to_string()),
                None,
                None,
            )
            .await
            .unwrap();

        let account = manager
            .import_external_account_json(
                r#"{
                    "account_id": "shared-upstream",
                    "chatgpt_account_id": "shared-upstream",
                    "access_token": "access-token",
                    "refresh_token": "new-refresh",
                    "email": "new@example.com",
                    "name": "new@example.com"
                }"#,
                None,
            )
            .await
            .unwrap();

        assert_ne!(account.id, "shared-upstream");
        assert_eq!(account.login, "new@example.com");

        let status = manager.get_status().await;
        assert_eq!(status.accounts.len(), 2);
        assert_eq!(
            status.default_account_id.as_deref(),
            Some("shared-upstream")
        );
        assert!(status
            .accounts
            .iter()
            .any(|account| account.id == "shared-upstream" && account.login == "old@example.com"));
        assert!(status
            .accounts
            .iter()
            .any(|stored| stored.id == account.id && stored.login == "new@example.com"));
    }

    #[tokio::test]
    async fn test_import_external_account_json_display_name_survives_native_sync() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        manager
            .import_external_account_json(
                r#"{
                    "account_id": "external-acc",
                    "access_token": "access-token",
                    "refresh_token": "refresh-token",
                    "email": "raw@example.com"
                }"#,
                Some("企业号"),
            )
            .await
            .unwrap();

        manager
            .upsert_native_account_snapshot(
                NativeCodexAccountSnapshot {
                    account_id: "external-acc".to_string(),
                    email: Some("real@example.com".to_string()),
                    id_token: None,
                    refresh_token: "new-refresh-token".to_string(),
                    access_token: Some("new-access-token".to_string()),
                    native_auth: build_native_auth_json(
                        "external-acc",
                        "new-access-token",
                        "new-refresh-token",
                        None,
                    ),
                },
                false,
            )
            .await
            .unwrap();

        let accounts = manager.list_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].login, "企业号");
    }

    #[tokio::test]
    async fn test_remove_account() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        manager
            .add_account_internal(
                "acc-123".to_string(),
                "rt".to_string(),
                None,
                Some("a@example.com".to_string()),
                None,
                None,
            )
            .await
            .unwrap();
        manager
            .add_account_internal(
                "acc-456".to_string(),
                "rt2".to_string(),
                None,
                Some("b@example.com".to_string()),
                None,
                None,
            )
            .await
            .unwrap();

        manager.remove_account("acc-123").await.unwrap();
        let accounts = manager.list_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "acc-456");
    }
}
