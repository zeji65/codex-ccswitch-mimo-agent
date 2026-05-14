//! Codex (OpenAI) Provider Adapter
//!
//! 默认透传 OpenAI API；部分第三方 provider 可通过 apiFormat 走轻量转换链。
//!
//! ## 客户端检测
//! 支持检测官方 Codex 客户端 (codex_vscode, codex_cli_rs)

use super::{AuthInfo, AuthStrategy, ProviderAdapter};
use crate::provider::Provider;
use crate::proxy::error::ProxyError;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

/// 官方 Codex 客户端 User-Agent 正则
#[allow(dead_code)]
static CODEX_CLIENT_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(codex_vscode|codex_cli_rs)/[\d.]+").unwrap());

/// Codex 适配器
pub struct CodexAdapter;

pub fn get_codex_api_format(provider: &Provider) -> &'static str {
    provider
        .meta
        .as_ref()
        .and_then(|meta| meta.api_format.as_deref())
        .map(|format| match format {
            "anthropic" => "anthropic",
            "openai_chat" => "openai_chat",
            _ => "openai_responses",
        })
        .unwrap_or("openai_responses")
}

fn should_strip_tools_for_openai_chat_bridge(provider: &Provider) -> bool {
    provider
        .meta
        .as_ref()
        .and_then(|meta| meta.provider_type.as_deref())
        == Some("litellm_gateway")
}

impl CodexAdapter {
    pub fn new() -> Self {
        Self
    }

    /// 检测是否为官方 Codex 客户端
    ///
    /// 匹配 User-Agent 模式: `^(codex_vscode|codex_cli_rs)/[\d.]+`
    #[allow(dead_code)]
    pub fn is_official_client(user_agent: &str) -> bool {
        CODEX_CLIENT_REGEX.is_match(user_agent)
    }

    /// 从 Provider 配置中提取 API Key
    fn extract_key(&self, provider: &Provider) -> Option<String> {
        // 1. 尝试从 env 中获取
        if let Some(env) = provider.settings_config.get("env") {
            if let Some(key) = env.get("OPENAI_API_KEY").and_then(|v| v.as_str()) {
                return Some(key.to_string());
            }
        }

        // 2. 尝试从 auth 中获取 (Codex CLI 格式)
        if let Some(auth) = provider.settings_config.get("auth") {
            if let Some(key) = auth.get("OPENAI_API_KEY").and_then(|v| v.as_str()) {
                return Some(key.to_string());
            }
        }

        // 3. 尝试直接获取
        if let Some(key) = provider
            .settings_config
            .get("apiKey")
            .or_else(|| provider.settings_config.get("api_key"))
            .and_then(|v| v.as_str())
        {
            return Some(key.to_string());
        }

        // 4. 尝试从 config 对象中获取
        if let Some(config) = provider.settings_config.get("config") {
            if let Some(key) = config
                .get("api_key")
                .or_else(|| config.get("apiKey"))
                .and_then(|v| v.as_str())
            {
                return Some(key.to_string());
            }
        }

        None
    }

    fn extract_configured_model(provider: &Provider) -> Option<String> {
        fn non_empty(value: Option<&str>) -> Option<String> {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        }

        provider
            .settings_config
            .get("config")
            .and_then(|config| {
                config
                    .as_str()
                    .and_then(crate::codex_config::extract_codex_toml_model)
                    .or_else(|| non_empty(config.get("model").and_then(|value| value.as_str())))
            })
            .or_else(|| {
                non_empty(
                    provider
                        .settings_config
                        .get("model")
                        .and_then(|value| value.as_str()),
                )
            })
            .or_else(|| {
                provider.settings_config.get("env").and_then(|env| {
                    non_empty(env.get("OPENAI_MODEL").and_then(|value| value.as_str())).or_else(
                        || non_empty(env.get("CODEX_MODEL").and_then(|value| value.as_str())),
                    )
                })
            })
    }

    fn apply_configured_model(mut body: Value, provider: &Provider) -> Value {
        if let Some(model) = Self::extract_configured_model(provider) {
            if body.get("model").and_then(|value| value.as_str()) != Some(model.as_str()) {
                log::debug!("[Codex] 覆盖第三方上游模型为供应商配置模型: {model}");
            }
            body["model"] = serde_json::json!(model);
        }
        body
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "Codex"
    }

    fn extract_base_url(&self, provider: &Provider) -> Result<String, ProxyError> {
        // 1. 尝试直接获取 base_url 字段
        if let Some(url) = provider
            .settings_config
            .get("base_url")
            .and_then(|v| v.as_str())
        {
            return Ok(url.trim_end_matches('/').to_string());
        }

        // 2. 尝试 baseURL
        if let Some(url) = provider
            .settings_config
            .get("baseURL")
            .and_then(|v| v.as_str())
        {
            return Ok(url.trim_end_matches('/').to_string());
        }

        // 3. 尝试从 config 对象中获取
        if let Some(config) = provider.settings_config.get("config") {
            if let Some(url) = config.get("base_url").and_then(|v| v.as_str()) {
                return Ok(url.trim_end_matches('/').to_string());
            }

            // 尝试解析 TOML 字符串格式
            if let Some(config_str) = config.as_str() {
                if let Some(start) = config_str.find("base_url = \"") {
                    let rest = &config_str[start + 12..];
                    if let Some(end) = rest.find('"') {
                        return Ok(rest[..end].trim_end_matches('/').to_string());
                    }
                }
                if let Some(start) = config_str.find("base_url = '") {
                    let rest = &config_str[start + 12..];
                    if let Some(end) = rest.find('\'') {
                        return Ok(rest[..end].trim_end_matches('/').to_string());
                    }
                }
            }
        }

        Err(ProxyError::ConfigError(
            "Codex Provider 缺少 base_url 配置".to_string(),
        ))
    }

    fn extract_auth(&self, provider: &Provider) -> Option<AuthInfo> {
        let strategy = if matches!(get_codex_api_format(provider), "anthropic") {
            AuthStrategy::Anthropic
        } else {
            AuthStrategy::Bearer
        };
        self.extract_key(provider)
            .map(|key| AuthInfo::new(key, strategy))
    }

    fn build_url(&self, base_url: &str, endpoint: &str) -> String {
        let base_trimmed = base_url.trim_end_matches('/');
        let endpoint_trimmed = endpoint.trim_start_matches('/');

        // OpenAI/Codex 的 base_url 可能是：
        // - 纯 origin: https://api.openai.com  (需要自动补 /v1)
        // - 已含 /v1: https://api.openai.com/v1 (直接拼接)
        // - 自定义前缀: https://xxx/openai (不添加 /v1，直接拼接)

        // 检查 base_url 是否已经包含 /v1
        let already_has_v1 = base_trimmed.ends_with("/v1");

        // 检查是否是纯 origin（没有路径部分）
        let origin_only = match base_trimmed.split_once("://") {
            Some((_scheme, rest)) => !rest.contains('/'),
            None => !base_trimmed.contains('/'),
        };

        let mut url = if already_has_v1 {
            // 已经有 /v1，直接拼接
            format!("{base_trimmed}/{endpoint_trimmed}")
        } else if origin_only {
            // 纯 origin，添加 /v1
            format!("{base_trimmed}/v1/{endpoint_trimmed}")
        } else {
            // 自定义前缀，不添加 /v1，直接拼接
            format!("{base_trimmed}/{endpoint_trimmed}")
        };

        // 去除重复的 /v1/v1（可能由 base_url 与 endpoint 都带版本导致）
        while url.contains("/v1/v1") {
            url = url.replace("/v1/v1", "/v1");
        }

        url
    }

    fn get_auth_headers(&self, auth: &AuthInfo) -> Vec<(http::HeaderName, http::HeaderValue)> {
        match auth.strategy {
            AuthStrategy::Anthropic => vec![
                (
                    http::HeaderName::from_static("x-api-key"),
                    http::HeaderValue::from_str(&auth.api_key).unwrap(),
                ),
                (
                    http::HeaderName::from_static("api-key"),
                    http::HeaderValue::from_str(&auth.api_key).unwrap(),
                ),
                (
                    http::HeaderName::from_static("anthropic-version"),
                    http::HeaderValue::from_static("2023-06-01"),
                ),
            ],
            _ => {
                let bearer = format!("Bearer {}", auth.api_key);
                vec![(
                    http::HeaderName::from_static("authorization"),
                    http::HeaderValue::from_str(&bearer).unwrap(),
                )]
            }
        }
    }

    fn needs_transform(&self, provider: &Provider) -> bool {
        matches!(get_codex_api_format(provider), "anthropic" | "openai_chat")
    }

    fn transform_request(&self, body: Value, provider: &Provider) -> Result<Value, ProxyError> {
        match get_codex_api_format(provider) {
            "anthropic" => {
                let body = Self::apply_configured_model(body, provider);
                if body.get("input").is_some() {
                    super::transform_responses::responses_request_to_anthropic(body)
                } else {
                    super::transform::openai_to_anthropic(body)
                }
            }
            "openai_chat" => {
                let mut body = Self::apply_configured_model(body, provider);
                if should_strip_tools_for_openai_chat_bridge(provider) {
                    if let Some(obj) = body.as_object_mut() {
                        obj.remove("tools");
                        obj.remove("tool_choice");
                        obj.remove("parallel_tool_calls");
                    }
                }
                if body.get("input").is_some() {
                    super::transform_responses::responses_request_to_openai_chat(body)
                } else {
                    Ok(body)
                }
            }
            _ => Ok(body),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_provider(config: serde_json::Value) -> Provider {
        Provider {
            id: "test".to_string(),
            name: "Test Codex".to_string(),
            settings_config: config,
            website_url: None,
            category: Some("codex".to_string()),
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    #[test]
    fn test_extract_base_url_direct() {
        let adapter = CodexAdapter::new();
        let provider = create_provider(json!({
            "base_url": "https://api.openai.com/v1"
        }));

        let url = adapter.extract_base_url(&provider).unwrap();
        assert_eq!(url, "https://api.openai.com/v1");
    }

    #[test]
    fn test_extract_auth_from_auth_field() {
        let adapter = CodexAdapter::new();
        let provider = create_provider(json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test-key-12345678"
            }
        }));

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.api_key, "sk-test-key-12345678");
        assert_eq!(auth.strategy, AuthStrategy::Bearer);
    }

    #[test]
    fn test_extract_auth_uses_anthropic_strategy_for_bridge_provider() {
        let adapter = CodexAdapter::new();
        let mut provider = create_provider(json!({
            "auth": {
                "OPENAI_API_KEY": "mimo-test-key-12345678"
            }
        }));
        provider.meta = Some(crate::provider::ProviderMeta {
            api_format: Some("anthropic".to_string()),
            ..Default::default()
        });

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.api_key, "mimo-test-key-12345678");
        assert_eq!(auth.strategy, AuthStrategy::Anthropic);
    }

    #[test]
    fn test_get_auth_headers_uses_x_api_key_for_anthropic_bridge() {
        let adapter = CodexAdapter::new();
        let auth = AuthInfo::new(
            "mimo-test-key-12345678".to_string(),
            AuthStrategy::Anthropic,
        );

        let headers = adapter.get_auth_headers(&auth);
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].0, http::HeaderName::from_static("x-api-key"));
        assert_eq!(
            headers[0].1,
            http::HeaderValue::from_static("mimo-test-key-12345678")
        );
        assert_eq!(headers[1].0, http::HeaderName::from_static("api-key"));
        assert_eq!(
            headers[2].0,
            http::HeaderName::from_static("anthropic-version")
        );
    }

    #[test]
    fn test_anthropic_bridge_overrides_client_model_with_provider_model() {
        let adapter = CodexAdapter::new();
        let mut provider = create_provider(json!({
            "config": r#"model_provider = "xiaomi_mimo"
model = "mimo-v2-pro"

[model_providers.xiaomi_mimo]
base_url = "https://api.xiaomimimo.com/anthropic"
"#
        }));
        provider.meta = Some(crate::provider::ProviderMeta {
            api_format: Some("anthropic".to_string()),
            ..Default::default()
        });

        let body = json!({
            "model": "gpt-5.3-codex",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "pong?" }]
            }],
            "stream": true
        });

        let result = adapter.transform_request(body, &provider).unwrap();

        assert_eq!(result["model"], json!("mimo-v2-pro"));
        assert_eq!(result["messages"][0]["role"], json!("user"));
    }

    #[test]
    fn test_extract_auth_from_env() {
        let adapter = CodexAdapter::new();
        let provider = create_provider(json!({
            "env": {
                "OPENAI_API_KEY": "sk-env-key-12345678"
            }
        }));

        let auth = adapter.extract_auth(&provider).unwrap();
        assert_eq!(auth.api_key, "sk-env-key-12345678");
    }

    #[test]
    fn test_build_url() {
        let adapter = CodexAdapter::new();
        let url = adapter.build_url("https://api.openai.com/v1", "/responses");
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    #[test]
    fn test_build_url_origin_adds_v1() {
        let adapter = CodexAdapter::new();
        let url = adapter.build_url("https://api.openai.com", "/responses");
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    #[test]
    fn test_build_url_custom_prefix_no_v1() {
        let adapter = CodexAdapter::new();
        let url = adapter.build_url("https://example.com/openai", "/responses");
        assert_eq!(url, "https://example.com/openai/responses");
    }

    #[test]
    fn test_build_url_dedup_v1() {
        let adapter = CodexAdapter::new();
        // base_url 已包含 /v1，endpoint 也包含 /v1
        let url = adapter.build_url("https://www.packyapi.com/v1", "/v1/responses");
        assert_eq!(url, "https://www.packyapi.com/v1/responses");
    }

    // 官方客户端检测测试
    #[test]
    fn test_is_official_client_vscode() {
        assert!(CodexAdapter::is_official_client("codex_vscode/1.0.0"));
        assert!(CodexAdapter::is_official_client("codex_vscode/2.3.4"));
        assert!(CodexAdapter::is_official_client("codex_vscode/0.1"));
    }

    #[test]
    fn test_is_official_client_cli() {
        assert!(CodexAdapter::is_official_client("codex_cli_rs/1.0.0"));
        assert!(CodexAdapter::is_official_client("codex_cli_rs/0.5.2"));
    }

    #[test]
    fn test_is_not_official_client() {
        assert!(!CodexAdapter::is_official_client("Mozilla/5.0"));
        assert!(!CodexAdapter::is_official_client("curl/7.68.0"));
        assert!(!CodexAdapter::is_official_client("python-requests/2.25.1"));
        assert!(!CodexAdapter::is_official_client("codex_other/1.0.0"));
        assert!(!CodexAdapter::is_official_client(""));
    }

    #[test]
    fn test_is_official_client_partial_match() {
        // 必须从开头匹配
        assert!(!CodexAdapter::is_official_client("some codex_vscode/1.0.0"));
        assert!(!CodexAdapter::is_official_client(
            "prefix_codex_cli_rs/1.0.0"
        ));
    }
}
