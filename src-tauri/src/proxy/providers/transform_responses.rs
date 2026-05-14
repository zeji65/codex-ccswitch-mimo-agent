//! OpenAI Responses API 格式转换模块
//!
//! 实现 Anthropic Messages ↔ OpenAI Responses API 格式转换。
//! Responses API 是 OpenAI 2025 年推出的新一代 API，采用扁平化的 input/output 结构。
//!
//! 与 Chat Completions 的主要差异：
//! - tool_use/tool_result 从 message content 中"提升"为顶层 input item
//! - system prompt 使用 `instructions` 字段而非 system role message
//! - usage 字段命名与 Anthropic 一致 (input_tokens/output_tokens)

use crate::proxy::error::ProxyError;
use serde_json::{json, Value};

pub(crate) const CODEX_ANTHROPIC_HISTORY_KEY: &str = "_cc_switch_anthropic_history";
const DEFAULT_ANTHROPIC_MAX_TOKENS: u64 = 4096;

/// Anthropic 请求 → OpenAI Responses 请求
///
/// `cache_key`: optional prompt_cache_key to inject for improved cache routing
/// `is_codex_oauth`: 当目标后端是 ChatGPT Plus/Pro 反代 (`chatgpt.com/backend-api/codex`) 时为 true。
/// 该后端强制要求 `store: false`，并要求 `include` 包含 `reasoning.encrypted_content`
/// 以便在无服务端状态下保持多轮 reasoning 上下文。
pub fn anthropic_to_responses(
    body: Value,
    cache_key: Option<&str>,
    is_codex_oauth: bool,
) -> Result<Value, ProxyError> {
    let mut result = json!({});

    // NOTE: 模型映射由上游统一处理（proxy::model_mapper），格式转换层只做结构转换。
    if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
        result["model"] = json!(model);
    }

    // system → instructions (Responses API 使用 instructions 字段)
    if let Some(system) = body.get("system") {
        let instructions = if let Some(text) = system.as_str() {
            text.to_string()
        } else if let Some(arr) = system.as_array() {
            arr.iter()
                .filter_map(|msg| msg.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n")
        } else {
            String::new()
        };
        if !instructions.is_empty() {
            result["instructions"] = json!(instructions);
        }
    }

    // messages → input
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        let input = convert_messages_to_input(msgs)?;
        result["input"] = json!(input);
    }

    // max_tokens → max_output_tokens (Responses API uses max_output_tokens for all models)
    if let Some(v) = body.get("max_tokens") {
        result["max_output_tokens"] = v.clone();
    }

    // 直接透传的参数
    if let Some(v) = body.get("temperature") {
        result["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        result["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stream") {
        result["stream"] = v.clone();
    }

    // Map Anthropic thinking → OpenAI Responses reasoning.effort
    if let Some(model_name) = body.get("model").and_then(|m| m.as_str()) {
        if super::transform::supports_reasoning_effort(model_name) {
            if let Some(effort) = super::transform::resolve_reasoning_effort(&body) {
                result["reasoning"] = json!({ "effort": effort });
            }
        }
    }

    // stop_sequences → 丢弃 (Responses API 不支持)

    // 转换 tools (过滤 BatchTool)
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let response_tools: Vec<Value> = tools
            .iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) != Some("BatchTool"))
            .filter_map(|t| {
                let name = super::transform::normalize_function_name(
                    t.get("name").and_then(|n| n.as_str()),
                )?;
                Some(json!({
                    "type": "function",
                    "name": name,
                    "description": t.get("description"),
                    "parameters": super::transform::clean_schema(
                        t.get("input_schema").cloned().unwrap_or(json!({}))
                    )
                }))
            })
            .collect();

        if !response_tools.is_empty() {
            result["tools"] = json!(response_tools);
        }
    }

    if let Some(v) = body.get("tool_choice") {
        result["tool_choice"] = map_tool_choice_to_responses(v);
    }

    // Inject prompt_cache_key for improved cache routing on OpenAI-compatible endpoints
    if let Some(key) = cache_key {
        result["prompt_cache_key"] = json!(key);
    }

    // Codex OAuth (ChatGPT Plus/Pro 反代) 特殊协议约束：
    // 整体依据：OpenAI 官方 codex-rs 的 `ResponsesApiRequest` 结构体
    // (codex-rs/codex-api/src/common.rs) 是 ChatGPT 反代后端的协议契约。
    // 任何不在该结构体里的字段都可能被 ChatGPT 后端以
    // "Unsupported parameter: ..." 400 拒绝；任何在结构体里的必填字段
    // 都需要在请求体里出现。
    //
    // 字段处理：
    // - store: 必须显式为 false（ChatGPT 消费级后端不允许服务端持久化）
    // - include: 必须包含 "reasoning.encrypted_content"，
    //   否则多轮 reasoning 中间态会丢失（无服务端状态 + 无加密回传 = 上下文断链）
    // - max_output_tokens / temperature / top_p: 必须删除
    //   （codex-rs 结构体根本没有这三个字段，OpenAI 自己的客户端不发它们）
    // - instructions / tools / parallel_tool_calls: 必填字段，缺则兜底默认值
    //   （cc-switch 的 transform 当前是"条件写入"，可能产生缺失）
    // - stream: 必须永远 true（codex-rs 硬编码 true，且 cc-switch 的
    //   SSE 解析层只处理流式响应，强制覆盖避免客户端误传 false）
    if is_codex_oauth {
        result["store"] = json!(false);

        const REASONING_MARKER: &str = "reasoning.encrypted_content";
        let mut includes: Vec<Value> = body
            .get("include")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if !includes
            .iter()
            .any(|v| v.as_str() == Some(REASONING_MARKER))
        {
            includes.push(json!(REASONING_MARKER));
        }
        result["include"] = json!(includes);

        if let Some(obj) = result.as_object_mut() {
            // —— 删除 ChatGPT 反代不接受的字段 ——
            obj.remove("max_output_tokens");
            obj.remove("temperature");
            obj.remove("top_p");

            // —— 兜底必填字段（or_insert：客户端送了什么就保留，否则注入默认值）——
            obj.entry("instructions".to_string()).or_insert(json!(""));
            obj.entry("tools".to_string()).or_insert(json!([]));
            obj.entry("parallel_tool_calls".to_string())
                .or_insert(json!(false));

            // —— 强制覆盖 stream = true ——
            // 即便客户端误传 stream:false 也要覆盖，因为 codex-rs 永远 true，
            // 且 cc-switch SSE 解析层只支持流式响应。
            obj.insert("stream".to_string(), json!(true));
        }
    }

    Ok(result)
}

/// OpenAI Responses 请求 → Anthropic Messages 请求
pub fn responses_request_to_anthropic(body: Value) -> Result<Value, ProxyError> {
    let mut result = json!({});

    if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
        result["model"] = json!(model);
    }

    if let Some(instructions) = body.get("instructions").and_then(|v| v.as_str()) {
        if !instructions.is_empty() {
            result["system"] = json!(instructions);
        }
    }

    result["messages"] = json!(extract_anthropic_messages_from_responses_request(&body)?);

    result["max_tokens"] = resolve_responses_max_tokens_for_anthropic(&body);
    if let Some(v) = body.get("temperature") {
        result["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        result["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stream") {
        result["stream"] = v.clone();
    }

    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let anthropic_tools: Vec<Value> = tools
            .iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) != Some("image"))
            .filter_map(|tool| {
                let name = super::transform::normalize_function_name(
                    tool.get("name").and_then(|n| n.as_str()),
                )?;
                Some(json!({
                    "name": name,
                    "description": tool.get("description"),
                    "input_schema": normalize_responses_tool_schema_for_anthropic(tool)
                }))
            })
            .collect();
        if !anthropic_tools.is_empty() {
            result["tools"] = json!(anthropic_tools);
        }
    }

    if let Some(tool_choice) = body.get("tool_choice") {
        result["tool_choice"] = map_tool_choice_to_anthropic(tool_choice);
    }

    Ok(result)
}

pub fn responses_request_to_openai_chat(body: Value) -> Result<Value, ProxyError> {
    let anthropic = responses_request_to_anthropic(body)?;
    super::transform::anthropic_to_openai(anthropic)
}

pub fn openai_chat_response_to_responses(body: Value) -> Result<Value, ProxyError> {
    let id = body
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("resp_cc_switch_chat")
        .to_string();
    let model = body
        .get("model")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = body
        .get("created")
        .and_then(|value| value.as_i64())
        .unwrap_or_else(|| chrono::Utc::now().timestamp());

    let mut output = Vec::new();
    if let Some(choice) = body
        .get("choices")
        .and_then(|value| value.as_array())
        .and_then(|choices| choices.first())
    {
        if let Some(message) = choice.get("message") {
            if let Some(content) = message.get("content").and_then(|value| value.as_str()) {
                if !content.is_empty() {
                    output.push(json!({
                        "id": format!("{id}_message_0"),
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": content,
                            "annotations": []
                        }]
                    }));
                }
            }

            if let Some(tool_calls) = message.get("tool_calls").and_then(|value| value.as_array()) {
                for (index, call) in tool_calls.iter().enumerate() {
                    let function = call.get("function").unwrap_or(&Value::Null);
                    let name = function
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    output.push(json!({
                        "id": call
                            .get("id")
                            .and_then(|value| value.as_str())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("fc_{index}")),
                        "type": "function_call",
                        "status": "completed",
                        "call_id": call
                            .get("id")
                            .and_then(|value| value.as_str())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("call_{index}")),
                        "name": name,
                        "arguments": function
                            .get("arguments")
                            .and_then(|value| value.as_str())
                            .unwrap_or("{}")
                    }));
                }
            }
        }
    }

    Ok(json!({
        "id": id,
        "object": "response",
        "created_at": created_at,
        "model": model,
        "status": "completed",
        "output": output,
        "usage": build_responses_usage_from_openai_chat(body.get("usage"))
    }))
}

pub(crate) fn build_responses_usage_from_openai_chat(usage: Option<&Value>) -> Value {
    let input_tokens = usage
        .and_then(|value| value.get("prompt_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|value| value.get("completion_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let total_tokens = usage
        .and_then(|value| value.get("total_tokens"))
        .and_then(|value| value.as_u64())
        .unwrap_or(input_tokens + output_tokens);

    json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens
    })
}

fn resolve_responses_max_tokens_for_anthropic(body: &Value) -> Value {
    for key in ["max_output_tokens", "max_tokens", "max_completion_tokens"] {
        if let Some(value) = body.get(key).filter(|value| !value.is_null()) {
            return value.clone();
        }
    }

    json!(DEFAULT_ANTHROPIC_MAX_TOKENS)
}

fn normalize_responses_tool_schema_for_anthropic(tool: &Value) -> Value {
    let raw_schema = tool
        .get("parameters")
        .or_else(|| tool.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let cleaned = super::transform::clean_schema(raw_schema);

    match cleaned {
        Value::Object(mut obj) => {
            let schema_type = obj.get("type");
            let type_is_object = schema_type.and_then(|value| value.as_str()) == Some("object");
            let type_missing_or_null = schema_type.map(|value| value.is_null()).unwrap_or(true);

            if type_is_object || type_missing_or_null {
                obj.insert("type".to_string(), json!("object"));
                if !obj.get("properties").is_some_and(|value| value.is_object()) {
                    obj.insert("properties".to_string(), json!({}));
                }
                Value::Object(obj)
            } else {
                json!({ "type": "object", "properties": {} })
            }
        }
        _ => json!({ "type": "object", "properties": {} }),
    }
}

pub(crate) fn extract_anthropic_messages_from_responses_request(
    body: &Value,
) -> Result<Vec<Value>, ProxyError> {
    let mut messages = Vec::new();

    if let Some(history) = body
        .get(CODEX_ANTHROPIC_HISTORY_KEY)
        .and_then(|value| value.as_array())
    {
        messages.extend(history.iter().cloned());
    }

    if let Some(input) = body.get("input").and_then(|value| value.as_array()) {
        messages.extend(convert_responses_input_to_messages(input)?);
    }

    if messages.is_empty() {
        return Err(ProxyError::InvalidRequest(
            "Codex Responses 请求没有 input，且 previous_response_id 未命中本地上下文缓存"
                .to_string(),
        ));
    }

    Ok(messages)
}

fn map_tool_choice_to_responses(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(_) => tool_choice.clone(),
        Value::Object(obj) => match obj.get("type").and_then(|t| t.as_str()) {
            // Anthropic "any" means at least one tool call is required
            Some("any") => json!("required"),
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            // Anthropic forced tool -> Responses function tool selector
            Some("tool") => {
                let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                json!({
                    "type": "function",
                    "name": name
                })
            }
            _ => tool_choice.clone(),
        },
        _ => tool_choice.clone(),
    }
}

pub(crate) fn map_responses_stop_reason(
    status: Option<&str>,
    has_tool_use: bool,
    incomplete_reason: Option<&str>,
) -> Option<&'static str> {
    status.map(|s| match s {
        "completed" if has_tool_use => "tool_use",
        "incomplete"
            if matches!(
                incomplete_reason,
                Some("max_output_tokens") | Some("max_tokens")
            ) || incomplete_reason.is_none() =>
        {
            "max_tokens"
        }
        _ => "end_turn",
    })
}

/// Build Anthropic-style usage JSON from Responses API usage, including cache tokens.
///
/// Priority order:
/// 1. OpenAI nested details (`input_tokens_details.cached_tokens`, `prompt_tokens_details.cached_tokens`) as initial value
/// 2. Direct Anthropic-style fields (`cache_read_input_tokens`, `cache_creation_input_tokens`) override if present
pub(crate) fn build_anthropic_usage_from_responses(usage: Option<&Value>) -> Value {
    let u = match usage {
        Some(v) if !v.is_null() => v,
        _ => {
            return json!({
                "input_tokens": 0,
                "output_tokens": 0
            })
        }
    };

    let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

    let mut result = json!({
        "input_tokens": input,
        "output_tokens": output
    });

    // Step 1: OpenAI nested details as fallback
    // OpenAI Responses API: input_tokens_details.cached_tokens
    if let Some(cached) = u
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
    {
        result["cache_read_input_tokens"] = json!(cached);
    }
    // OpenAI standard: prompt_tokens_details.cached_tokens
    if let Some(cached) = u
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
    {
        if result.get("cache_read_input_tokens").is_none() {
            result["cache_read_input_tokens"] = json!(cached);
        }
    }

    // Step 2: Direct Anthropic-style fields override (authoritative if present)
    if let Some(v) = u.get("cache_read_input_tokens") {
        result["cache_read_input_tokens"] = v.clone();
    }
    if let Some(v) = u.get("cache_creation_input_tokens") {
        result["cache_creation_input_tokens"] = v.clone();
    }

    result
}

pub(crate) fn build_responses_usage_from_anthropic(usage: Option<&Value>) -> Value {
    let u = match usage {
        Some(v) if !v.is_null() => v,
        _ => {
            return json!({
                "input_tokens": 0,
                "output_tokens": 0,
                "total_tokens": 0
            })
        }
    };

    let cached_tokens = u
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| u.get("prompt_cache_hit_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    let prompt_cache_miss_tokens = u
        .get("prompt_cache_miss_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let input_tokens = u
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(cached_tokens + prompt_cache_miss_tokens);
    let output_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

    let mut result = json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens
    });

    if cached_tokens > 0 {
        result["cache_read_input_tokens"] = json!(cached_tokens);
        result["input_tokens_details"] = json!({ "cached_tokens": cached_tokens });
    }

    if let Some(v) = u.get("cache_creation_input_tokens") {
        result["cache_creation_input_tokens"] = v.clone();
    }

    result
}

/// 将 Anthropic messages 数组转换为 Responses API input 数组
///
/// 核心转换逻辑：
/// - user/assistant 的 text 内容 → 对应 role 的 message item
/// - tool_use 从 assistant message 中"提升"为独立的 function_call item
/// - tool_result 从 user message 中"提升"为独立的 function_call_output item
/// - thinking blocks → 丢弃
fn convert_messages_to_input(messages: &[Value]) -> Result<Vec<Value>, ProxyError> {
    let mut input = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = msg.get("content");

        match content {
            // 字符串内容
            Some(Value::String(text)) => {
                let content_type = if role == "assistant" {
                    "output_text"
                } else {
                    "input_text"
                };
                input.push(json!({
                    "role": role,
                    "content": [{ "type": content_type, "text": text }]
                }));
            }

            // 数组内容（多模态/工具调用）
            Some(Value::Array(blocks)) => {
                let mut message_content = Vec::new();

                for block in blocks {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                    match block_type {
                        "text" => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                let content_type = if role == "assistant" {
                                    "output_text"
                                } else {
                                    "input_text"
                                };
                                // OpenAI Responses API does not accept Anthropic cache_control
                                // under input[].content[].
                                message_content.push(json!({ "type": content_type, "text": text }));
                            }
                        }

                        "image" => {
                            if let Some(source) = block.get("source") {
                                let media_type = source
                                    .get("media_type")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("image/png");
                                let data =
                                    source.get("data").and_then(|d| d.as_str()).unwrap_or("");
                                message_content.push(json!({
                                    "type": "input_image",
                                    "image_url": format!("data:{media_type};base64,{data}")
                                }));
                            }
                        }

                        "tool_use" => {
                            // 先刷新已累积的消息内容
                            if !message_content.is_empty() {
                                input.push(json!({
                                    "role": role,
                                    "content": message_content.clone()
                                }));
                                message_content.clear();
                            }

                            // 提升为独立的 function_call item
                            let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let arguments = block.get("input").cloned().unwrap_or(json!({}));

                            input.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": serde_json::to_string(&arguments).unwrap_or_default()
                            }));
                        }

                        "tool_result" => {
                            // 先刷新已累积的消息内容
                            if !message_content.is_empty() {
                                input.push(json!({
                                    "role": role,
                                    "content": message_content.clone()
                                }));
                                message_content.clear();
                            }

                            // 提升为独立的 function_call_output item
                            let call_id = block
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            let output = match block.get("content") {
                                Some(Value::String(s)) => s.clone(),
                                Some(v) => serde_json::to_string(v).unwrap_or_default(),
                                None => String::new(),
                            };

                            input.push(json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": output
                            }));
                        }

                        "thinking" => {
                            // 丢弃 thinking blocks（与 openai_chat 一致）
                        }

                        _ => {}
                    }
                }

                // 刷新剩余的消息内容
                if !message_content.is_empty() {
                    input.push(json!({
                        "role": role,
                        "content": message_content
                    }));
                }
            }

            _ => {
                // 无内容或 null
                input.push(json!({ "role": role }));
            }
        }
    }

    Ok(input)
}

fn convert_responses_input_to_messages(input: &[Value]) -> Result<Vec<Value>, ProxyError> {
    let mut messages = Vec::new();

    for item in input {
        if let Some(item_type) = item.get("type").and_then(|v| v.as_str()) {
            match item_type {
                "function_call" => {
                    let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or_else(|| json!({}));
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": call_id,
                            "name": name,
                            "input": arguments
                        }]
                    }));
                    continue;
                }
                "function_call_output" => {
                    let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let output = item.get("output").cloned().unwrap_or(json!(""));
                    let anthropic_output = match output {
                        Value::String(s) => serde_json::from_str::<Value>(&s).unwrap_or(json!(s)),
                        other => other,
                    };
                    messages.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": call_id,
                            "content": anthropic_output
                        }]
                    }));
                    continue;
                }
                _ => {}
            }
        }

        let raw_role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let role = normalize_responses_role_for_anthropic(raw_role);
        let content = item.get("content");
        let anthropic_content = convert_responses_content_to_blocks(content)?;
        messages.push(json!({
            "role": role,
            "content": anthropic_content
        }));
    }

    Ok(messages)
}

fn normalize_responses_role_for_anthropic(role: &str) -> &'static str {
    match role {
        "assistant" => "assistant",
        _ => "user",
    }
}

fn convert_responses_content_to_blocks(content: Option<&Value>) -> Result<Vec<Value>, ProxyError> {
    let mut blocks = Vec::new();

    match content {
        Some(Value::String(text)) => {
            if !text.is_empty() {
                blocks.push(json!({ "type": "text", "text": text }));
            }
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "input_text" | "output_text" | "text" => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                    }
                    "input_image" => {
                        if let Some(image_url) = part.get("image_url").and_then(|v| v.as_str()) {
                            if let Some((media_type, data)) = parse_data_url(image_url) {
                                blocks.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": data
                                    }
                                }));
                            }
                        }
                    }
                    "refusal" => {
                        if let Some(text) = part
                            .get("refusal")
                            .or_else(|| part.get("text"))
                            .and_then(|v| v.as_str())
                        {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Value::Object(_)) => {
            return Err(ProxyError::TransformError(
                "Unsupported Responses content object".to_string(),
            ));
        }
        _ => {}
    }

    Ok(blocks)
}

fn parse_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(",")?;
    let media_type = meta.strip_suffix(";base64").unwrap_or(meta).to_string();
    Some((media_type, data.to_string()))
}

fn map_tool_choice_to_anthropic(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(value) => match value.as_str() {
            "required" => json!({ "type": "any" }),
            "auto" => json!({ "type": "auto" }),
            "none" => json!({ "type": "none" }),
            _ => tool_choice.clone(),
        },
        Value::Object(obj) => match obj.get("type").and_then(|t| t.as_str()) {
            Some("function") => json!({
                "type": "tool",
                "name": obj.get("name").and_then(|n| n.as_str()).unwrap_or("")
            }),
            _ => tool_choice.clone(),
        },
        _ => tool_choice.clone(),
    }
}

/// Anthropic 响应 → OpenAI Responses 响应
pub fn anthropic_response_to_responses(body: Value) -> Result<Value, ProxyError> {
    let content = body
        .get("content")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ProxyError::TransformError("No content in anthropic response".to_string())
        })?;

    let mut output = Vec::new();
    let mut message_content = Vec::new();
    let mut combined_text = String::new();

    let flush_message = |output: &mut Vec<Value>, message_content: &mut Vec<Value>| {
        if !message_content.is_empty() {
            output.push(json!({
                "id": format!("{}_message_{}", body.get("id").and_then(|v| v.as_str()).unwrap_or("resp"), output.len()),
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": message_content.clone()
            }));
            message_content.clear();
        }
    };

    for block in content {
        match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" => {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        combined_text.push_str(text);
                        message_content.push(json!({
                            "type": "output_text",
                            "text": text,
                            "annotations": []
                        }));
                    }
                }
            }
            "tool_use" => {
                flush_message(&mut output, &mut message_content);
                let call_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = block.get("input").cloned().unwrap_or(json!({}));
                output.push(json!({
                    "id": format!("fc_{call_id}"),
                    "type": "function_call",
                    "status": "completed",
                    "call_id": call_id,
                    "name": name,
                    "arguments": serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string())
                }));
            }
            "thinking" => {
                flush_message(&mut output, &mut message_content);
                if let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) {
                    if !thinking.is_empty() {
                        output.push(json!({
                            "id": format!("rs_{}", output.len()),
                            "type": "reasoning",
                            "status": "completed",
                            "summary": [{
                                "type": "summary_text",
                                "text": thinking
                            }]
                        }));
                    }
                }
            }
            _ => {}
        }
    }
    flush_message(&mut output, &mut message_content);

    let stop_reason = body.get("stop_reason").and_then(|v| v.as_str());
    let status = if stop_reason == Some("max_tokens") {
        "incomplete"
    } else {
        "completed"
    };

    let mut result = json!({
        "id": body.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "model": body.get("model").and_then(|v| v.as_str()).unwrap_or(""),
        "status": status,
        "output": output,
        "usage": build_responses_usage_from_anthropic(body.get("usage"))
    });

    if !combined_text.is_empty() {
        result["output_text"] = json!(combined_text);
    }

    if status == "incomplete" {
        result["incomplete_details"] = json!({
            "reason": "max_output_tokens"
        });
    }

    Ok(result)
}

/// OpenAI Responses 响应 → Anthropic 响应
pub fn responses_to_anthropic(body: Value) -> Result<Value, ProxyError> {
    let output = body
        .get("output")
        .and_then(|o| o.as_array())
        .ok_or_else(|| ProxyError::TransformError("No output in response".to_string()))?;

    let mut content = Vec::new();

    let mut has_tool_use = false;
    for item in output {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match item_type {
            "message" => {
                if let Some(msg_content) = item.get("content").and_then(|c| c.as_array()) {
                    for block in msg_content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if block_type == "output_text" {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    content.push(json!({"type": "text", "text": text}));
                                }
                            }
                        } else if block_type == "refusal" {
                            if let Some(refusal) = block.get("refusal").and_then(|t| t.as_str()) {
                                if !refusal.is_empty() {
                                    content.push(json!({"type": "text", "text": refusal}));
                                }
                            }
                        }
                    }
                }
            }

            "function_call" => {
                let call_id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args_str = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

                content.push(json!({
                    "type": "tool_use",
                    "id": call_id,
                    "name": name,
                    "input": input
                }));
                has_tool_use = true;
            }

            "reasoning" => {
                // 映射 reasoning summary → thinking block
                if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
                    let thinking_text: String = summary
                        .iter()
                        .filter_map(|s| {
                            if s.get("type").and_then(|t| t.as_str()) == Some("summary_text") {
                                s.get("text").and_then(|t| t.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    if !thinking_text.is_empty() {
                        content.push(json!({
                            "type": "thinking",
                            "thinking": thinking_text
                        }));
                    }
                }
            }

            _ => {}
        }
    }

    // status → stop_reason
    let stop_reason = map_responses_stop_reason(
        body.get("status").and_then(|s| s.as_str()),
        has_tool_use,
        body.pointer("/incomplete_details/reason")
            .and_then(|r| r.as_str()),
    );

    let usage_json = build_anthropic_usage_from_responses(body.get("usage"));

    let result = json!({
        "id": body.get("id").and_then(|i| i.as_str()).unwrap_or(""),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": body.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": usage_json
    });

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_to_responses_simple() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["model"], "gpt-4o");
        assert_eq!(result["max_output_tokens"], 1024);
        assert_eq!(result["input"][0]["role"], "user");
        assert_eq!(result["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(result["input"][0]["content"][0]["text"], "Hello");
        // stop_sequences should not appear
        assert!(result.get("stop_sequences").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_with_system_string() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "system": "You are a helpful assistant.",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["instructions"], "You are a helpful assistant.");
        // system should not appear in input
        assert_eq!(result["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_anthropic_to_responses_with_system_array() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "Part 1"},
                {"type": "text", "text": "Part 2"}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["instructions"], "Part 1\n\nPart 2");
    }

    #[test]
    fn test_anthropic_to_responses_with_tools() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather info",
                "input_schema": {"type": "object", "properties": {"location": {"type": "string"}}}
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["tools"][0]["type"], "function");
        assert_eq!(result["tools"][0]["name"], "get_weather");
        assert!(result["tools"][0].get("parameters").is_some());
        // input_schema should not appear
        assert!(result["tools"][0].get("input_schema").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_tool_choice_any_to_required() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tool_choice": {"type": "any"}
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["tool_choice"], "required");
    }

    #[test]
    fn test_anthropic_to_responses_tool_choice_tool_to_function() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tool_choice": {"type": "tool", "name": "get_weather"}
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["tool_choice"]["type"], "function");
        assert_eq!(result["tool_choice"]["name"], "get_weather");
    }

    #[test]
    fn test_anthropic_to_responses_tool_use_lifting() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Let me check"},
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        let input_arr = result["input"].as_array().unwrap();

        // Should produce: assistant message (text) + function_call item
        assert_eq!(input_arr.len(), 2);

        // First: assistant message with output_text
        assert_eq!(input_arr[0]["role"], "assistant");
        assert_eq!(input_arr[0]["content"][0]["type"], "output_text");
        assert_eq!(input_arr[0]["content"][0]["text"], "Let me check");

        // Second: function_call item (lifted from message)
        assert_eq!(input_arr[1]["type"], "function_call");
        assert_eq!(input_arr[1]["call_id"], "call_123");
        assert_eq!(input_arr[1]["name"], "get_weather");
    }

    #[test]
    fn test_anthropic_to_responses_tool_result_lifting() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "call_123", "content": "Sunny, 25°C"}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        let input_arr = result["input"].as_array().unwrap();

        // Should produce: function_call_output item (lifted)
        assert_eq!(input_arr.len(), 1);
        assert_eq!(input_arr[0]["type"], "function_call_output");
        assert_eq!(input_arr[0]["call_id"], "call_123");
        assert_eq!(input_arr[0]["output"], "Sunny, 25°C");
    }

    #[test]
    fn test_anthropic_to_responses_thinking_discarded() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "Let me think..."},
                    {"type": "text", "text": "The answer is 42"}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        let input_arr = result["input"].as_array().unwrap();

        // thinking should be discarded, only text remains
        assert_eq!(input_arr.len(), 1);
        assert_eq!(input_arr[0]["content"][0]["type"], "output_text");
        assert_eq!(input_arr[0]["content"][0]["text"], "The answer is 42");
    }

    #[test]
    fn test_anthropic_to_responses_image() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "What is this?"},
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc123"}}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        let content = result["input"][0]["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "data:image/png;base64,abc123");
    }

    #[test]
    fn test_responses_to_anthropic_simple() {
        let input = json!({
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "id": "msg_123",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["id"], "resp_123");
        assert_eq!(result["type"], "message");
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "Hello!");
        assert_eq!(result["stop_reason"], "end_turn");
        assert_eq!(result["usage"]["input_tokens"], 10);
        assert_eq!(result["usage"]["output_tokens"], 5);
    }

    #[test]
    fn test_responses_to_anthropic_with_function_call() {
        let input = json!({
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "function_call",
                "id": "fc_123",
                "call_id": "call_123",
                "name": "get_weather",
                "arguments": "{\"location\": \"Tokyo\"}",
                "status": "completed"
            }],
            "usage": {"input_tokens": 10, "output_tokens": 15}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "tool_use");
        assert_eq!(result["content"][0]["id"], "call_123");
        assert_eq!(result["content"][0]["name"], "get_weather");
        assert_eq!(result["content"][0]["input"]["location"], "Tokyo");
        assert_eq!(result["stop_reason"], "tool_use");
    }

    #[test]
    fn test_responses_to_anthropic_with_refusal_block() {
        let input = json!({
            "id": "resp_123",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "refusal", "refusal": "I can't help with that."}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "I can't help with that.");
        assert_eq!(result["stop_reason"], "end_turn");
    }

    #[test]
    fn test_responses_to_anthropic_with_reasoning() {
        let input = json!({
            "id": "resp_123",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o",
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_123",
                    "summary": [
                        {"type": "summary_text", "text": "Thinking about the problem..."}
                    ]
                },
                {
                    "type": "message",
                    "id": "msg_123",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "The answer is 42"}]
                }
            ],
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });

        let result = responses_to_anthropic(input).unwrap();
        // Should have thinking + text
        assert_eq!(result["content"][0]["type"], "thinking");
        assert_eq!(
            result["content"][0]["thinking"],
            "Thinking about the problem..."
        );
        assert_eq!(result["content"][1]["type"], "text");
        assert_eq!(result["content"][1]["text"], "The answer is 42");
    }

    #[test]
    fn test_responses_to_anthropic_incomplete_status() {
        let input = json!({
            "id": "resp_123",
            "status": "incomplete",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Partial..."}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 4096}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["stop_reason"], "max_tokens");
    }

    #[test]
    fn test_responses_to_anthropic_incomplete_non_token_reason() {
        let input = json!({
            "id": "resp_123",
            "status": "incomplete",
            "incomplete_details": {"reason": "content_filter"},
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Blocked"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 1}
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["stop_reason"], "end_turn");
    }

    #[test]
    fn test_model_passthrough() {
        let input = json!({
            "model": "o3-mini",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["model"], "o3-mini");
    }

    #[test]
    fn test_anthropic_to_responses_with_cache_key() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, Some("my-provider-id"), false).unwrap();
        assert_eq!(result["prompt_cache_key"], "my-provider-id");
    }

    #[test]
    fn test_anthropic_to_responses_strip_cache_control_on_tools() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {"type": "object"},
                "cache_control": {"type": "ephemeral"}
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert!(result["tools"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_strip_cache_control_on_text() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Hello", "cache_control": {"type": "ephemeral"}}
                ]
            }]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert!(result["input"][0]["content"][0]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn test_responses_to_anthropic_with_cache_tokens() {
        let input = json!({
            "id": "resp_123",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "input_tokens_details": {
                    "cached_tokens": 80
                }
            }
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["usage"]["input_tokens"], 100);
        assert_eq!(result["usage"]["output_tokens"], 50);
        assert_eq!(result["usage"]["cache_read_input_tokens"], 80);
    }

    #[test]
    fn test_responses_to_anthropic_with_direct_cache_fields() {
        let input = json!({
            "id": "resp_123",
            "status": "completed",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 60,
                "cache_creation_input_tokens": 20
            }
        });

        let result = responses_to_anthropic(input).unwrap();
        assert_eq!(result["usage"]["cache_read_input_tokens"], 60);
        assert_eq!(result["usage"]["cache_creation_input_tokens"], 20);
    }

    #[test]
    fn test_anthropic_to_responses_o_series_uses_max_output_tokens() {
        // Responses API always uses max_output_tokens, even for o-series models
        let input = json!({
            "model": "o3-mini",
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["max_output_tokens"], 4096);
        assert!(result.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_responses_output_config_max_sets_reasoning_xhigh() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "output_config": {"effort": "max"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn test_responses_output_config_takes_priority_over_thinking() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "output_config": {"effort": "low"},
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["reasoning"]["effort"], "low");
    }

    #[test]
    fn test_responses_thinking_enabled_small_budget_sets_reasoning_low() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["reasoning"]["effort"], "low");
    }

    #[test]
    fn test_responses_thinking_enabled_medium_budget_sets_reasoning_medium() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 8000},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["reasoning"]["effort"], "medium");
    }

    #[test]
    fn test_responses_thinking_enabled_large_budget_sets_reasoning_high() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 32000},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_responses_thinking_adaptive_sets_reasoning_xhigh() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert_eq!(result["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn test_responses_non_reasoning_model_no_reasoning() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();
        assert!(result.get("reasoning").is_none());
    }

    // ==================== Codex OAuth (ChatGPT 反代) 协议约束 ====================

    #[test]
    fn test_anthropic_to_responses_codex_oauth_sets_store_and_include() {
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        // store 必须显式为 false（ChatGPT 后端拒绝 true）
        assert_eq!(result["store"], json!(false));

        // include 必须包含 reasoning.encrypted_content（无服务端状态下保持多轮 reasoning）
        assert_eq!(result["include"], json!(["reasoning.encrypted_content"]));
    }

    #[test]
    fn test_anthropic_to_responses_non_codex_omits_store_and_include() {
        // 回归护栏：is_codex_oauth=false 时，行为必须与今日字节级一致
        // —— 不写 store、不写 include，OpenRouter / Azure / OpenAI 付费 API 路径不受影响
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();

        assert!(result.get("store").is_none());
        assert!(result.get("include").is_none());
    }

    #[test]
    fn test_anthropic_to_responses_codex_oauth_preserves_existing_include() {
        // 客户端预置了 include：union 保留原有项 + 添加 marker，不重复
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}],
            "include": ["something.else", "reasoning.encrypted_content"]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();
        let includes = result["include"]
            .as_array()
            .expect("include should be array");

        // 原有项必须保留
        assert!(includes
            .iter()
            .any(|v| v.as_str() == Some("something.else")));
        // marker 必须存在
        assert!(includes
            .iter()
            .any(|v| v.as_str() == Some("reasoning.encrypted_content")));
        // 不重复：marker 只出现一次
        let marker_count = includes
            .iter()
            .filter(|v| v.as_str() == Some("reasoning.encrypted_content"))
            .count();
        assert_eq!(marker_count, 1, "marker 不应被重复添加（idempotent 失败）");
    }

    #[test]
    fn test_anthropic_to_responses_codex_oauth_strips_max_output_tokens() {
        // ChatGPT Plus/Pro 反代不接受 max_output_tokens（OpenAI 官方 codex-rs 的
        // ResponsesApiRequest 结构体里也没有这个字段），必须删除，否则服务端 400：
        // "Unsupported parameter: max_output_tokens"
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        assert!(
            result.get("max_output_tokens").is_none(),
            "Codex OAuth 路径必须删除 max_output_tokens"
        );
    }

    #[test]
    fn test_anthropic_to_responses_non_codex_keeps_max_output_tokens() {
        // 回归护栏：非 Codex OAuth 路径必须保留 max_output_tokens
        // —— OpenAI 付费 Responses API / Azure 等仍然依赖这个字段
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();

        assert_eq!(result["max_output_tokens"], json!(1024));
    }

    // ==================== 第二轮：P0 + P1 字段对齐 ====================

    #[test]
    fn test_codex_oauth_strips_temperature() {
        // P0: ChatGPT 反代不接受 temperature
        // 依据：OpenAI 官方 codex-rs 的 ResponsesApiRequest 结构体根本没有这个字段
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "temperature": 0.7,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        assert!(
            result.get("temperature").is_none(),
            "Codex OAuth 路径必须删除 temperature"
        );
    }

    #[test]
    fn test_codex_oauth_strips_top_p() {
        // P0: ChatGPT 反代不接受 top_p
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "top_p": 0.9,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        assert!(
            result.get("top_p").is_none(),
            "Codex OAuth 路径必须删除 top_p"
        );
    }

    #[test]
    fn test_codex_oauth_defaults_required_fields_when_absent() {
        // P1: 极简输入（无 system / 无 tools / 无 stream），断言四个必填字段都被注入默认值
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        assert_eq!(
            result["instructions"],
            json!(""),
            "instructions 缺失时应兜底为空字符串"
        );
        assert_eq!(result["tools"], json!([]), "tools 缺失时应兜底为空数组");
        assert_eq!(
            result["parallel_tool_calls"],
            json!(false),
            "parallel_tool_calls 应兜底为 false"
        );
        assert_eq!(result["stream"], json!(true), "stream 应被强制设为 true");
    }

    #[test]
    fn test_codex_oauth_preserves_existing_instructions_and_tools() {
        // P1: 客户端送了 system 和 tools，应保留原值，不被默认值覆盖
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "system": "You are a helpful assistant",
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {
                    "type": "object",
                    "properties": {"city": {"type": "string"}}
                }
            }],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        assert_eq!(
            result["instructions"],
            json!("You are a helpful assistant"),
            "client 已送的 instructions 必须保留"
        );

        let tools = result["tools"].as_array().expect("tools 应为数组");
        assert_eq!(tools.len(), 1, "client 已送的 tools 必须保留");
        assert_eq!(tools[0]["name"], json!("get_weather"));
    }

    #[test]
    fn test_codex_oauth_forces_stream_true_even_when_client_sends_false() {
        // 即使客户端误传 stream:false，也要强制覆盖为 true
        // 依据：cc-switch SSE 解析层只支持流式响应
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "stream": false,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, true).unwrap();

        assert_eq!(
            result["stream"],
            json!(true),
            "Codex OAuth 路径下 stream 必须强制为 true"
        );
    }

    #[test]
    fn test_non_codex_keeps_temperature_and_top_p() {
        // 回归护栏：非 Codex OAuth 路径必须保留 temperature/top_p
        // —— 防止 P0 删除逻辑误扩散到 OpenRouter / Azure / 付费 OpenAI 路径
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "temperature": 0.7,
            "top_p": 0.9,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();

        assert_eq!(result["temperature"], json!(0.7));
        assert_eq!(result["top_p"], json!(0.9));
    }

    #[test]
    fn test_non_codex_does_not_inject_default_required_fields() {
        // 回归护栏：非 Codex OAuth 路径不应被 P1 默认值污染
        // —— OpenRouter / Azure / 付费 OpenAI 等保持原有"条件写入"语义
        let input = json!({
            "model": "gpt-5-codex",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_responses(input, None, false).unwrap();

        assert!(
            result.get("parallel_tool_calls").is_none(),
            "非 Codex OAuth 路径不应注入 parallel_tool_calls"
        );
        assert!(
            result.get("stream").is_none(),
            "非 Codex OAuth 路径不应注入 stream"
        );
        // instructions 和 tools 因为客户端没送，所以不应出现
        assert!(
            result.get("instructions").is_none(),
            "非 Codex OAuth 路径下 instructions 在客户端未送时不应被注入"
        );
        assert!(
            result.get("tools").is_none(),
            "非 Codex OAuth 路径下 tools 在客户端未送时不应被注入"
        );
    }

    #[test]
    fn test_responses_request_to_anthropic_maps_instructions_and_tools() {
        let input = json!({
            "model": "deepseek-v3.2",
            "instructions": "You are a careful coding assistant.",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "帮我看一下这个 diff" }]
            }],
            "tools": [{
                "type": "function",
                "name": "read_file",
                "description": "Read a file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }],
            "tool_choice": {
                "type": "function",
                "name": "read_file"
            },
            "stream": true
        });

        let result = responses_request_to_anthropic(input).unwrap();

        assert_eq!(result["model"], json!("deepseek-v3.2"));
        assert_eq!(result["max_tokens"], json!(DEFAULT_ANTHROPIC_MAX_TOKENS));
        assert_eq!(
            result["system"],
            json!("You are a careful coding assistant.")
        );
        assert_eq!(result["stream"], json!(true));
        assert_eq!(result["messages"][0]["role"], json!("user"));
        assert_eq!(result["messages"][0]["content"][0]["type"], json!("text"));
        assert_eq!(
            result["messages"][0]["content"][0]["text"],
            json!("帮我看一下这个 diff")
        );
        assert_eq!(result["tools"][0]["name"], json!("read_file"));
        assert_eq!(result["tool_choice"]["type"], json!("tool"));
        assert_eq!(result["tool_choice"]["name"], json!("read_file"));
    }

    #[test]
    fn test_responses_request_to_anthropic_fixes_null_tool_schema() {
        let input = json!({
            "model": "deepseek-v3.2",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "patch it" }]
            }],
            "tools": [{
                "type": "function",
                "name": "apply_patch",
                "description": "Apply a patch",
                "parameters": { "type": null }
            }],
            "stream": true
        });

        let result = responses_request_to_anthropic(input).unwrap();

        assert_eq!(result["tools"][0]["name"], json!("apply_patch"));
        assert_eq!(result["tools"][0]["input_schema"]["type"], json!("object"));
        assert_eq!(result["tools"][0]["input_schema"]["properties"], json!({}));
    }

    #[test]
    fn test_responses_request_to_anthropic_defaults_missing_tool_schema() {
        let input = json!({
            "model": "deepseek-v3.2",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "patch it" }]
            }],
            "tools": [{
                "type": "function",
                "name": "apply_patch",
                "description": "Apply a patch",
                "parameters": null
            }],
            "stream": true
        });

        let result = responses_request_to_anthropic(input).unwrap();

        assert_eq!(result["tools"][0]["input_schema"]["type"], json!("object"));
        assert_eq!(result["tools"][0]["input_schema"]["properties"], json!({}));
    }

    #[test]
    fn test_responses_request_to_anthropic_skips_empty_tool_names() {
        let input = json!({
            "model": "deepseek-v3.2",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "use tools" }]
            }],
            "tools": [
                {
                    "type": "function",
                    "name": "",
                    "description": "Invalid empty tool",
                    "parameters": { "type": "object" }
                },
                {
                    "type": "function",
                    "name": "valid/tool",
                    "description": "Valid after normalization",
                    "parameters": { "type": "object" }
                }
            ],
            "stream": true
        });

        let result = responses_request_to_anthropic(input).unwrap();

        let tools = result["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], json!("valid_tool"));
    }

    #[test]
    fn test_responses_request_to_anthropic_normalizes_codex_roles() {
        let input = json!({
            "model": "mimo-v2-pro",
            "input": [
                {
                    "role": "developer",
                    "content": [{ "type": "input_text", "text": "Be concise." }]
                },
                {
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "ping" }]
                },
                {
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "pong" }]
                }
            ],
            "stream": true
        });

        let result = responses_request_to_anthropic(input).unwrap();

        assert_eq!(result["messages"][0]["role"], json!("user"));
        assert_eq!(
            result["messages"][0]["content"][0]["text"],
            json!("Be concise.")
        );
        assert_eq!(result["messages"][1]["role"], json!("user"));
        assert_eq!(result["messages"][2]["role"], json!("assistant"));
    }

    #[test]
    fn test_responses_request_to_anthropic_maps_max_output_tokens() {
        let input = json!({
            "model": "deepseek-v3.2",
            "max_output_tokens": 1234,
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "ping" }]
            }]
        });

        let result = responses_request_to_anthropic(input).unwrap();

        assert_eq!(result["max_tokens"], json!(1234));
    }

    #[test]
    fn test_responses_request_to_anthropic_defaults_missing_max_tokens() {
        let input = json!({
            "model": "deepseek-v3.2",
            "input": [{
                "role": "user",
                "content": [{ "type": "input_text", "text": "ping" }]
            }]
        });

        let result = responses_request_to_anthropic(input).unwrap();

        assert_eq!(result["max_tokens"], json!(DEFAULT_ANTHROPIC_MAX_TOKENS));
    }

    #[test]
    fn test_anthropic_response_to_responses_maps_cache_usage_and_tool_calls() {
        let input = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "deepseek-v3.2",
            "content": [
                { "type": "text", "text": "先读文件。" },
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "read_file",
                    "input": { "path": "/tmp/demo.ts" }
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 120,
                "output_tokens": 18,
                "prompt_cache_hit_tokens": 80
            }
        });

        let result = anthropic_response_to_responses(input).unwrap();

        assert_eq!(result["id"], json!("msg_123"));
        assert_eq!(result["model"], json!("deepseek-v3.2"));
        assert_eq!(result["status"], json!("completed"));
        assert_eq!(result["output"][0]["type"], json!("message"));
        assert_eq!(
            result["output"][0]["content"][0]["type"],
            json!("output_text")
        );
        assert_eq!(result["output"][1]["type"], json!("function_call"));
        assert_eq!(result["output"][1]["call_id"], json!("toolu_1"));
        assert_eq!(result["output"][1]["name"], json!("read_file"));
        assert_eq!(result["usage"]["input_tokens"], json!(120));
        assert_eq!(result["usage"]["output_tokens"], json!(18));
        assert_eq!(result["usage"]["cache_read_input_tokens"], json!(80));
        assert_eq!(
            result["usage"]["input_tokens_details"]["cached_tokens"],
            json!(80)
        );
    }
}
