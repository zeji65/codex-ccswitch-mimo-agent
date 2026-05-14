//! OpenAI Responses API 流式转换模块
//!
//! 实现 Responses API SSE → Anthropic SSE 格式转换。
//!
//! Responses API 使用命名事件 (named events) 的生命周期模型：
//! response.created → output_item.added → content_part.added →
//! output_text.delta → content_part.done → output_item.done → response.completed
//!
//! 与 Chat Completions 的 delta chunk 模型完全不同，需要独立的状态机处理。

use super::transform_responses::{
    anthropic_response_to_responses, build_anthropic_usage_from_responses,
    build_responses_usage_from_anthropic, build_responses_usage_from_openai_chat,
    map_responses_stop_reason,
};
use crate::proxy::sse::{strip_sse_field, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub type AnthropicHistoryRecorder = Arc<dyn Fn(String, Vec<Value>) + Send + Sync>;

#[inline]
fn response_object_from_event(data: &Value) -> &Value {
    data.get("response").unwrap_or(data)
}

#[inline]
fn content_part_key(data: &Value) -> Option<String> {
    if let (Some(item_id), Some(content_index)) = (
        data.get("item_id").and_then(|v| v.as_str()),
        data.get("content_index").and_then(|v| v.as_u64()),
    ) {
        return Some(format!("part:{item_id}:{content_index}"));
    }
    if let (Some(output_index), Some(content_index)) = (
        data.get("output_index").and_then(|v| v.as_u64()),
        data.get("content_index").and_then(|v| v.as_u64()),
    ) {
        return Some(format!("part:out:{output_index}:{content_index}"));
    }
    None
}

#[inline]
fn tool_item_key_from_added(data: &Value, item: &Value) -> Option<String> {
    if let Some(item_id) = item.get("id").and_then(|v| v.as_str()) {
        return Some(format!("tool:{item_id}"));
    }
    if let Some(item_id) = data.get("item_id").and_then(|v| v.as_str()) {
        return Some(format!("tool:{item_id}"));
    }
    if let Some(output_index) = data.get("output_index").and_then(|v| v.as_u64()) {
        return Some(format!("tool:out:{output_index}"));
    }
    None
}

#[inline]
fn tool_item_key_from_event(data: &Value) -> Option<String> {
    if let Some(item_id) = data.get("item_id").and_then(|v| v.as_str()) {
        return Some(format!("tool:{item_id}"));
    }
    if let Some(output_index) = data.get("output_index").and_then(|v| v.as_u64()) {
        return Some(format!("tool:out:{output_index}"));
    }
    None
}

/// Resolve content index for a text/refusal content part event.
///
/// Uses `content_part_key` to look up or assign a stable index, falling back to
/// `fallback_open_index` when no key is available.
#[inline]
fn resolve_content_index(
    data: &Value,
    next_content_index: &mut u32,
    index_by_key: &mut HashMap<String, u32>,
    fallback_open_index: &mut Option<u32>,
) -> u32 {
    if let Some(k) = content_part_key(data) {
        if let Some(existing) = index_by_key.get(&k).copied() {
            existing
        } else {
            let assigned = *next_content_index;
            *next_content_index += 1;
            index_by_key.insert(k, assigned);
            assigned
        }
    } else if let Some(existing) = *fallback_open_index {
        existing
    } else {
        let assigned = *next_content_index;
        *next_content_index += 1;
        *fallback_open_index = Some(assigned);
        assigned
    }
}

/// 创建从 Responses API SSE 到 Anthropic SSE 的转换流
///
/// 状态机跟踪: message_id, current_model, has_sent_message_start, item/content index map
/// SSE 解析支持 named events (event: + data: 行)
pub fn create_anthropic_sse_stream_from_responses<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut message_id: Option<String> = None;
        let mut current_model: Option<String> = None;
        let mut has_sent_message_start = false;
        let mut has_tool_use = false;
        let mut next_content_index: u32 = 0;
        let mut index_by_key: HashMap<String, u32> = HashMap::new();
        let mut open_indices: HashSet<u32> = HashSet::new();
        let mut fallback_open_index: Option<u32> = None;
        let mut current_text_index: Option<u32> = None;
        let mut tool_index_by_item_id: HashMap<String, u32> = HashMap::new();
        let mut last_tool_index: Option<u32> = None;

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    // SSE 事件由 \n\n 分隔
                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }

                        // 解析 SSE 块：提取 event: 和 data: 行
                        let mut event_type: Option<String> = None;
                        let mut data_parts: Vec<String> = Vec::new();

                        for line in block.lines() {
                            if let Some(evt) = strip_sse_field(line, "event") {
                                event_type = Some(evt.trim().to_string());
                            } else if let Some(d) = strip_sse_field(line, "data") {
                                data_parts.push(d.to_string());
                            }
                        }

                        if data_parts.is_empty() {
                            continue;
                        }

                        let data_str = data_parts.join("\n");
                        let event_name = event_type.as_deref().unwrap_or("");

                        // 解析 JSON 数据
                        let data: Value = match serde_json::from_str(&data_str) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        log::debug!("[Claude/Responses] <<< SSE event: {event_name}");

                        match event_name {
                            // ================================================
                            // response.created → message_start
                            // ================================================
                            "response.created" => {
                                let response_obj = response_object_from_event(&data);
                                if let Some(id) = response_obj.get("id").and_then(|i| i.as_str()) {
                                    message_id = Some(id.to_string());
                                }
                                if let Some(model) =
                                    response_obj.get("model").and_then(|m| m.as_str())
                                {
                                    current_model = Some(model.to_string());
                                }

                                has_sent_message_start = true;
                                // Build usage with cache tokens if available
                                let start_usage = build_anthropic_usage_from_responses(
                                    response_obj.get("usage"),
                                );

                                let event = json!({
                                    "type": "message_start",
                                    "message": {
                                        "id": message_id.clone().unwrap_or_default(),
                                        "type": "message",
                                        "role": "assistant",
                                        "model": current_model.clone().unwrap_or_default(),
                                        "usage": start_usage
                                    }
                                });
                                let sse = format!("event: message_start\ndata: {}\n\n",
                                    serde_json::to_string(&event).unwrap_or_default());
                                log::debug!("[Claude/Responses] >>> Anthropic SSE: message_start");
                                yield Ok(Bytes::from(sse));
                            }

                            // ================================================
                            // response.content_part.added → content_block_start (text)
                            // ================================================
                            "response.content_part.added" => {
                                // 确保 message_start 已发送
                                if !has_sent_message_start {
                                    let start_event = json!({
                                        "type": "message_start",
                                        "message": {
                                            "id": message_id.clone().unwrap_or_default(),
                                            "type": "message",
                                            "role": "assistant",
                                            "model": current_model.clone().unwrap_or_default(),
                                            "usage": { "input_tokens": 0, "output_tokens": 0 }
                                        }
                                    });
                                    let sse = format!("event: message_start\ndata: {}\n\n",
                                        serde_json::to_string(&start_event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                    has_sent_message_start = true;
                                }

                                if let Some(part) = data.get("part") {
                                    let part_type = part.get("type").and_then(|t| t.as_str());
                                    if matches!(part_type, Some("output_text") | Some("refusal")) {
                                        let index = if let Some(index) = current_text_index {
                                            index
                                        } else {
                                            let index = resolve_content_index(
                                                &data,
                                                &mut next_content_index,
                                                &mut index_by_key,
                                                &mut fallback_open_index,
                                            );
                                            current_text_index = Some(index);
                                            index
                                        };

                                        if open_indices.contains(&index) {
                                            continue;
                                        }

                                        let event = json!({
                                            "type": "content_block_start",
                                            "index": index,
                                            "content_block": {
                                                "type": "text",
                                                "text": ""
                                            }
                                        });
                                        let sse = format!("event: content_block_start\ndata: {}\n\n",
                                            serde_json::to_string(&event).unwrap_or_default());
                                        yield Ok(Bytes::from(sse));
                                        open_indices.insert(index);
                                    }
                                }
                            }

                            // ================================================
                            // response.output_text.delta → content_block_delta (text_delta)
                            // ================================================
                            "response.output_text.delta" => {
                                if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
                                    let index = if let Some(index) = current_text_index {
                                        index
                                    } else {
                                        let index = resolve_content_index(
                                            &data,
                                            &mut next_content_index,
                                            &mut index_by_key,
                                            &mut fallback_open_index,
                                        );
                                        current_text_index = Some(index);
                                        index
                                    };

                                    if !open_indices.contains(&index) {
                                        let start_event = json!({
                                            "type": "content_block_start",
                                            "index": index,
                                            "content_block": {
                                                "type": "text",
                                                "text": ""
                                            }
                                        });
                                        let start_sse = format!("event: content_block_start\ndata: {}\n\n",
                                            serde_json::to_string(&start_event).unwrap_or_default());
                                        yield Ok(Bytes::from(start_sse));
                                        open_indices.insert(index);
                                    }
                                    let event = json!({
                                        "type": "content_block_delta",
                                        "index": index,
                                        "delta": {
                                            "type": "text_delta",
                                            "text": delta
                                        }
                                    });
                                    let sse = format!("event: content_block_delta\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                }
                            }

                            // ================================================
                            // response.refusal.delta → content_block_delta (text_delta)
                            // ================================================
                            "response.refusal.delta" => {
                                if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
                                    let index = if let Some(index) = current_text_index {
                                        index
                                    } else {
                                        let index = resolve_content_index(
                                            &data,
                                            &mut next_content_index,
                                            &mut index_by_key,
                                            &mut fallback_open_index,
                                        );
                                        current_text_index = Some(index);
                                        index
                                    };

                                    if !open_indices.contains(&index) {
                                        let start_event = json!({
                                            "type": "content_block_start",
                                            "index": index,
                                            "content_block": {
                                                "type": "text",
                                                "text": ""
                                            }
                                        });
                                        let start_sse = format!("event: content_block_start\ndata: {}\n\n",
                                            serde_json::to_string(&start_event).unwrap_or_default());
                                        yield Ok(Bytes::from(start_sse));
                                        open_indices.insert(index);
                                    }

                                    let event = json!({
                                        "type": "content_block_delta",
                                        "index": index,
                                        "delta": {
                                            "type": "text_delta",
                                            "text": delta
                                        }
                                    });
                                    let sse = format!("event: content_block_delta\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                }
                            }

                            // ================================================
                            // response.content_part.done → content_block_stop
                            // ================================================
                            "response.content_part.done" => {}

                            // ================================================
                            // response.output_item.added (function_call) → content_block_start (tool_use)
                            // ================================================
                            "response.output_item.added" => {
                                if let Some(item) = data.get("item") {
                                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if item_type == "function_call" {
                                        has_tool_use = true;
                                        if let Some(index) = current_text_index.take() {
                                            if open_indices.remove(&index) {
                                                let stop_event = json!({
                                                    "type": "content_block_stop",
                                                    "index": index
                                                });
                                                let stop_sse = format!("event: content_block_stop\ndata: {}\n\n",
                                                    serde_json::to_string(&stop_event).unwrap_or_default());
                                                yield Ok(Bytes::from(stop_sse));
                                            }
                                            if fallback_open_index == Some(index) {
                                                fallback_open_index = None;
                                            }
                                        }
                                        // 确保 message_start 已发送
                                        if !has_sent_message_start {
                                            let start_event = json!({
                                                "type": "message_start",
                                                "message": {
                                                    "id": message_id.clone().unwrap_or_default(),
                                                    "type": "message",
                                                    "role": "assistant",
                                                    "model": current_model.clone().unwrap_or_default(),
                                                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                                                }
                                            });
                                            let sse = format!("event: message_start\ndata: {}\n\n",
                                                serde_json::to_string(&start_event).unwrap_or_default());
                                            yield Ok(Bytes::from(sse));
                                            has_sent_message_start = true;
                                        }

                                        let call_id = item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                        let index = if let Some(k) = tool_item_key_from_added(&data, item) {
                                            if let Some(existing) = index_by_key.get(&k).copied() {
                                                existing
                                            } else {
                                                let assigned = next_content_index;
                                                next_content_index += 1;
                                                index_by_key.insert(k, assigned);
                                                assigned
                                            }
                                        } else {
                                            let assigned = next_content_index;
                                            next_content_index += 1;
                                            assigned
                                        };
                                        if let Some(item_id) = item
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                            .or_else(|| data.get("item_id").and_then(|v| v.as_str()))
                                        {
                                            tool_index_by_item_id.insert(item_id.to_string(), index);
                                        }
                                        last_tool_index = Some(index);

                                        if open_indices.contains(&index) {
                                            continue;
                                        }

                                        let event = json!({
                                            "type": "content_block_start",
                                            "index": index,
                                            "content_block": {
                                                "type": "tool_use",
                                                "id": call_id,
                                                "name": name
                                            }
                                        });
                                        let sse = format!("event: content_block_start\ndata: {}\n\n",
                                            serde_json::to_string(&event).unwrap_or_default());
                                        yield Ok(Bytes::from(sse));
                                        open_indices.insert(index);
                                    }
                                    // message type output_item.added is handled via content_part.added
                                }
                            }

                            // ================================================
                            // response.function_call_arguments.delta → content_block_delta (input_json_delta)
                            // ================================================
                            "response.function_call_arguments.delta" => {
                                if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
                                    let item_id = data.get("item_id").and_then(|v| v.as_str());
                                    let index = if let Some(id) = item_id {
                                        tool_index_by_item_id.get(id).copied()
                                    } else {
                                        None
                                    }
                                    .or_else(|| {
                                        tool_item_key_from_event(&data)
                                            .and_then(|k| index_by_key.get(&k).copied())
                                    })
                                    .or(last_tool_index)
                                    .unwrap_or_else(|| {
                                        let assigned = next_content_index;
                                        next_content_index += 1;
                                        assigned
                                    });

                                    if !open_indices.contains(&index) {
                                        let start_event = json!({
                                            "type": "content_block_start",
                                            "index": index,
                                            "content_block": {
                                                "type": "tool_use",
                                                "id": data
                                                    .get("call_id")
                                                    .and_then(|v| v.as_str())
                                                    .or(item_id)
                                                    .unwrap_or(""),
                                                "name": data
                                                    .get("name")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                            }
                                        });
                                        let start_sse = format!("event: content_block_start\ndata: {}\n\n",
                                            serde_json::to_string(&start_event).unwrap_or_default());
                                        yield Ok(Bytes::from(start_sse));
                                        open_indices.insert(index);
                                    }

                                    let event = json!({
                                        "type": "content_block_delta",
                                        "index": index,
                                        "delta": {
                                            "type": "input_json_delta",
                                            "partial_json": delta
                                        }
                                    });
                                    let sse = format!("event: content_block_delta\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                }
                            }

                            // ================================================
                            // response.function_call_arguments.done → content_block_stop
                            // ================================================
                            "response.function_call_arguments.done" => {
                                let item_id = data.get("item_id").and_then(|v| v.as_str());
                                let index = if let Some(id) = item_id {
                                    tool_index_by_item_id.get(id).copied()
                                } else {
                                    None
                                }
                                .or_else(|| {
                                    tool_item_key_from_event(&data)
                                        .and_then(|k| index_by_key.get(&k).copied())
                                })
                                .or(last_tool_index);
                                if let Some(index) = index {
                                    if !open_indices.remove(&index) {
                                        continue;
                                    }
                                    let event = json!({
                                        "type": "content_block_stop",
                                        "index": index
                                    });
                                    let sse = format!("event: content_block_stop\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                    if let Some(item_id) = item_id {
                                        tool_index_by_item_id.remove(item_id);
                                    }
                                }
                            }

                            // ================================================
                            // response.refusal.done → content_block_stop
                            // ================================================
                            "response.refusal.done" => {
                                let index = current_text_index.take().or_else(|| {
                                    let key = content_part_key(&data);
                                    if let Some(k) = key {
                                        index_by_key.get(&k).copied()
                                    } else {
                                        fallback_open_index
                                    }
                                });
                                if let Some(index) = index {
                                    if !open_indices.remove(&index) {
                                        continue;
                                    }
                                    let event = json!({
                                        "type": "content_block_stop",
                                        "index": index
                                    });
                                    let sse = format!("event: content_block_stop\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                    if fallback_open_index == Some(index) {
                                        fallback_open_index = None;
                                    }
                                }
                            }

                            // ================================================
                            // response.reasoning.delta → content_block_delta (thinking_delta)
                            // ================================================
                            "response.reasoning.delta" => {
                                if let Some(delta) = data
                                    .get("delta")
                                    .or_else(|| data.get("text"))
                                    .and_then(|d| d.as_str())
                                {
                                    if let Some(index) = current_text_index.take() {
                                        if open_indices.remove(&index) {
                                            let stop_event = json!({
                                                "type": "content_block_stop",
                                                "index": index
                                            });
                                            let stop_sse = format!("event: content_block_stop\ndata: {}\n\n",
                                                serde_json::to_string(&stop_event).unwrap_or_default());
                                            yield Ok(Bytes::from(stop_sse));
                                        }
                                        if fallback_open_index == Some(index) {
                                            fallback_open_index = None;
                                        }
                                    }
                                    let index = resolve_content_index(
                                        &data,
                                        &mut next_content_index,
                                        &mut index_by_key,
                                        &mut fallback_open_index,
                                    );

                                    if !open_indices.contains(&index) {
                                        let start_event = json!({
                                            "type": "content_block_start",
                                            "index": index,
                                            "content_block": {
                                                "type": "thinking",
                                                "thinking": ""
                                            }
                                        });
                                        let start_sse = format!("event: content_block_start\ndata: {}\n\n",
                                            serde_json::to_string(&start_event).unwrap_or_default());
                                        yield Ok(Bytes::from(start_sse));
                                        open_indices.insert(index);
                                    }

                                    let event = json!({
                                        "type": "content_block_delta",
                                        "index": index,
                                        "delta": {
                                            "type": "thinking_delta",
                                            "thinking": delta
                                        }
                                    });
                                    let sse = format!("event: content_block_delta\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                }
                            }

                            // ================================================
                            // response.reasoning.done → content_block_stop
                            // ================================================
                            "response.reasoning.done" => {
                                let key = content_part_key(&data);
                                let index = if let Some(k) = key {
                                    index_by_key.get(&k).copied()
                                } else {
                                    fallback_open_index
                                };
                                if let Some(index) = index {
                                    if !open_indices.remove(&index) {
                                        continue;
                                    }
                                    let event = json!({
                                        "type": "content_block_stop",
                                        "index": index
                                    });
                                    let sse = format!("event: content_block_stop\ndata: {}\n\n",
                                        serde_json::to_string(&event).unwrap_or_default());
                                    yield Ok(Bytes::from(sse));
                                    if fallback_open_index == Some(index) {
                                        fallback_open_index = None;
                                    }
                                }
                            }

                            // ================================================
                            // response.completed → message_delta + message_stop
                            // ================================================
                            "response.completed" => {
                                let response_obj = response_object_from_event(&data);
                                let stop_reason = map_responses_stop_reason(
                                    response_obj.get("status").and_then(|s| s.as_str()),
                                    has_tool_use,
                                    response_obj
                                        .pointer("/incomplete_details/reason")
                                        .and_then(|r| r.as_str()),
                                );

                                // Best effort: close any dangling blocks before message_delta/message_stop.
                                if !open_indices.is_empty() {
                                    let mut remaining: Vec<u32> = open_indices.iter().copied().collect();
                                    remaining.sort_unstable();
                                    for index in remaining {
                                        let stop_event = json!({
                                            "type": "content_block_stop",
                                            "index": index
                                        });
                                        let stop_sse = format!("event: content_block_stop\ndata: {}\n\n",
                                            serde_json::to_string(&stop_event).unwrap_or_default());
                                        yield Ok(Bytes::from(stop_sse));
                                        open_indices.remove(&index);
                                    }
                                }
                                fallback_open_index = None;

                                let usage_json = response_obj.get("usage").map(|u| {
                                    build_anthropic_usage_from_responses(Some(u))
                                });

                                // Emit message_delta (with usage + stop_reason)
                                let delta_event = json!({
                                    "type": "message_delta",
                                    "delta": {
                                        "stop_reason": stop_reason,
                                        "stop_sequence": null
                                    },
                                    "usage": usage_json
                                });
                                let sse = format!("event: message_delta\ndata: {}\n\n",
                                    serde_json::to_string(&delta_event).unwrap_or_default());
                                log::debug!("[Claude/Responses] >>> Anthropic SSE: message_delta");
                                yield Ok(Bytes::from(sse));

                                // Emit message_stop
                                let stop_event = json!({"type": "message_stop"});
                                let stop_sse = format!("event: message_stop\ndata: {}\n\n",
                                    serde_json::to_string(&stop_event).unwrap_or_default());
                                log::debug!("[Claude/Responses] >>> Anthropic SSE: message_stop");
                                yield Ok(Bytes::from(stop_sse));
                            }

                            // Lifecycle events that don't need Anthropic counterparts.
                            // Listed explicitly so new events trigger a match-completeness review.
                            "response.output_text.done" => {
                                if let Some(index) = current_text_index.take() {
                                    if open_indices.remove(&index) {
                                        let stop_event = json!({
                                            "type": "content_block_stop",
                                            "index": index
                                        });
                                        let stop_sse = format!("event: content_block_stop\ndata: {}\n\n",
                                            serde_json::to_string(&stop_event).unwrap_or_default());
                                        yield Ok(Bytes::from(stop_sse));
                                    }
                                    if fallback_open_index == Some(index) {
                                        fallback_open_index = None;
                                    }
                                }
                            }
                            "response.output_item.done"
                            | "response.in_progress" => {}

                            // Any other unknown/future events — silently skip.
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    log::error!("Responses stream error: {e}");
                    let error_event = json!({
                        "type": "error",
                        "error": {
                            "type": "stream_error",
                            "message": format!("Stream error: {e}")
                        }
                    });
                    let sse = format!("event: error\ndata: {}\n\n",
                        serde_json::to_string(&error_event).unwrap_or_default());
                    yield Ok(Bytes::from(sse));
                    break;
                }
            }
        }
    }
}

/// 创建从 Anthropic SSE 到 OpenAI Responses SSE 的转换流
#[allow(dead_code)]
pub fn create_responses_sse_stream_from_anthropic<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    create_responses_sse_stream_from_anthropic_with_history(stream, Vec::new(), None)
}

pub fn create_responses_sse_stream_from_openai_chat<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut response_id = String::new();
        let mut model = String::new();
        let mut text = String::new();
        let mut started = false;
        let mut completed = false;
        let mut usage = json!({});

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }

                        let mut data_parts: Vec<String> = Vec::new();
                        for line in block.lines() {
                            if let Some(data) = strip_sse_field(line, "data") {
                                data_parts.push(data.to_string());
                            }
                        }
                        if data_parts.is_empty() {
                            continue;
                        }

                        let data_str = data_parts.join("\n");
                        if data_str.trim() == "[DONE]" {
                            completed = true;
                            break;
                        }

                        let event = match serde_json::from_str::<Value>(&data_str) {
                            Ok(value) => value,
                            Err(_) => continue,
                        };

                        if response_id.is_empty() {
                            response_id = event
                                .get("id")
                                .and_then(|value| value.as_str())
                                .unwrap_or("resp_cc_switch_chat")
                                .to_string();
                        }
                        if model.is_empty() {
                            model = event
                                .get("model")
                                .and_then(|value| value.as_str())
                                .unwrap_or("")
                                .to_string();
                        }
                        if let Some(event_usage) = event.get("usage") {
                            usage = event_usage.clone();
                        }

                        if !started {
                            let created = json!({
                                "type": "response.created",
                                "response": {
                                    "id": response_id.clone(),
                                    "object": "response",
                                    "created_at": chrono::Utc::now().timestamp(),
                                    "model": model.clone(),
                                    "status": "in_progress",
                                    "output": [],
                                    "usage": build_responses_usage_from_openai_chat(Some(&usage))
                                }
                            });
                            yield Ok(Bytes::from(format!("event: response.created\ndata: {}\n\n", serde_json::to_string(&created).unwrap_or_default())));

                            let item = json!({
                                "type": "response.output_item.added",
                                "output_index": 0,
                                "item": {
                                    "id": format!("{}_message_0", response_id),
                                    "type": "message",
                                    "status": "in_progress",
                                    "role": "assistant",
                                    "content": []
                                }
                            });
                            yield Ok(Bytes::from(format!("event: response.output_item.added\ndata: {}\n\n", serde_json::to_string(&item).unwrap_or_default())));

                            let part = json!({
                                "type": "response.content_part.added",
                                "output_index": 0,
                                "item_id": format!("{}_message_0", response_id),
                                "content_index": 0,
                                "part": {
                                    "type": "output_text",
                                    "text": "",
                                    "annotations": []
                                }
                            });
                            yield Ok(Bytes::from(format!("event: response.content_part.added\ndata: {}\n\n", serde_json::to_string(&part).unwrap_or_default())));
                            started = true;
                        }

                        if let Some(delta) = event
                            .get("choices")
                            .and_then(|value| value.as_array())
                            .and_then(|choices| choices.first())
                            .and_then(|choice| choice.get("delta"))
                            .and_then(|delta| delta.get("content"))
                            .and_then(|value| value.as_str())
                        {
                            if !delta.is_empty() {
                                text.push_str(delta);
                                let payload = json!({
                                    "type": "response.output_text.delta",
                                    "item_id": format!("{}_message_0", response_id),
                                    "output_index": 0,
                                    "content_index": 0,
                                    "delta": delta
                                });
                                yield Ok(Bytes::from(format!("event: response.output_text.delta\ndata: {}\n\n", serde_json::to_string(&payload).unwrap_or_default())));
                            }
                        }
                    }

                    if completed {
                        break;
                    }
                }
                Err(error) => {
                    yield Err(std::io::Error::other(error.to_string()));
                    break;
                }
            }
        }

        if !started {
            response_id = if response_id.is_empty() {
                "resp_cc_switch_chat".to_string()
            } else {
                response_id
            };
            let created = json!({
                "type": "response.created",
                "response": {
                    "id": response_id.clone(),
                    "object": "response",
                    "created_at": chrono::Utc::now().timestamp(),
                    "model": model.clone(),
                    "status": "in_progress",
                    "output": [],
                    "usage": build_responses_usage_from_openai_chat(Some(&usage))
                }
            });
            yield Ok(Bytes::from(format!("event: response.created\ndata: {}\n\n", serde_json::to_string(&created).unwrap_or_default())));
        }

        let item_id = format!("{}_message_0", response_id);
        let text_done = json!({
            "type": "response.output_text.done",
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "text": text
        });
        yield Ok(Bytes::from(format!("event: response.output_text.done\ndata: {}\n\n", serde_json::to_string(&text_done).unwrap_or_default())));

        let part_done = json!({
            "type": "response.content_part.done",
            "item_id": text_done["item_id"],
            "output_index": 0,
            "content_index": 0,
            "part": {
                "type": "output_text",
                "text": text_done["text"],
                "annotations": []
            }
        });
        yield Ok(Bytes::from(format!("event: response.content_part.done\ndata: {}\n\n", serde_json::to_string(&part_done).unwrap_or_default())));

        let item_done = json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "id": text_done["item_id"],
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": text_done["text"],
                    "annotations": []
                }]
            }
        });
        yield Ok(Bytes::from(format!("event: response.output_item.done\ndata: {}\n\n", serde_json::to_string(&item_done).unwrap_or_default())));

        let completed_response = json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
                "object": "response",
                "created_at": chrono::Utc::now().timestamp(),
                "model": model,
                "status": "completed",
                "output": [item_done["item"].clone()],
                "usage": build_responses_usage_from_openai_chat(Some(&usage))
            }
        });
        yield Ok(Bytes::from(format!("event: response.completed\ndata: {}\n\n", serde_json::to_string(&completed_response).unwrap_or_default())));
    }
}

pub fn create_responses_sse_stream_from_anthropic_with_history<
    E: std::error::Error + Send + 'static,
>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
    request_messages: Vec<Value>,
    history_recorder: Option<AnthropicHistoryRecorder>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut message_id = String::new();
        let mut model = String::new();
        let mut output_index: u32 = 0;
        let mut active_text: Option<(String, u32, String)> = None;
        let mut active_tool: Option<(String, String, String, String, u32)> = None;
        let mut active_reasoning: Option<(u32, String)> = None;
        let mut final_content: Vec<Value> = Vec::new();
        let mut raw_usage = json!({});
        let mut stop_reason: Option<String> = None;

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }

                        let mut event_name: Option<String> = None;
                        let mut data_parts: Vec<String> = Vec::new();
                        for line in block.lines() {
                            if let Some(evt) = strip_sse_field(line, "event") {
                                event_name = Some(evt.trim().to_string());
                            } else if let Some(data) = strip_sse_field(line, "data") {
                                data_parts.push(data.to_string());
                            }
                        }

                        if data_parts.is_empty() {
                            continue;
                        }

                        let data_str = data_parts.join("\n");
                        let event = match serde_json::from_str::<Value>(&data_str) {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let event_type = event_name
                            .as_deref()
                            .or_else(|| event.get("type").and_then(|v| v.as_str()))
                            .unwrap_or("");

                        match event_type {
                            "message_start" => {
                                let message = event.get("message").unwrap_or(&event);
                                message_id = message.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                model = message.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                raw_usage = message.get("usage").cloned().unwrap_or_else(|| json!({}));

                                let created = json!({
                                    "type": "response.created",
                                    "response": {
                                        "id": message_id.clone(),
                                        "object": "response",
                                        "created_at": chrono::Utc::now().timestamp(),
                                        "model": model.clone(),
                                        "status": "in_progress",
                                        "output": [],
                                        "usage": build_responses_usage_from_anthropic(Some(&raw_usage))
                                    }
                                });
                                yield Ok(Bytes::from(format!("event: response.created\ndata: {}\n\n", serde_json::to_string(&created).unwrap_or_default())));
                            }
                            "content_block_start" => {
                                let index = event.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                if let Some(content_block) = event.get("content_block") {
                                    match content_block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                                        "text" => {
                                            let item_id = format!("{}_message_{}", message_id, output_index);
                                            active_text = Some((item_id.clone(), index, String::new()));
                                            let item = json!({
                                                "type": "response.output_item.added",
                                                "output_index": output_index,
                                                "item": {
                                                    "id": item_id,
                                                    "type": "message",
                                                    "status": "in_progress",
                                                    "role": "assistant",
                                                    "content": []
                                                }
                                            });
                                            yield Ok(Bytes::from(format!("event: response.output_item.added\ndata: {}\n\n", serde_json::to_string(&item).unwrap_or_default())));

                                            let part = json!({
                                                "type": "response.content_part.added",
                                                "output_index": output_index,
                                                "item_id": active_text.as_ref().map(|v| v.0.clone()).unwrap_or_default(),
                                                "content_index": index,
                                                "part": {
                                                    "type": "output_text",
                                                    "text": "",
                                                    "annotations": []
                                                }
                                            });
                                            yield Ok(Bytes::from(format!("event: response.content_part.added\ndata: {}\n\n", serde_json::to_string(&part).unwrap_or_default())));
                                        }
                                        "tool_use" => {
                                            let call_id = content_block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let name = content_block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            let item_id = format!("fc_{call_id}");
                                            active_tool = Some((item_id.clone(), call_id.clone(), name.clone(), String::new(), output_index));
                                            let item = json!({
                                                "type": "response.output_item.added",
                                                "output_index": output_index,
                                                "item": {
                                                    "id": item_id,
                                                    "type": "function_call",
                                                    "status": "in_progress",
                                                    "call_id": call_id,
                                                    "name": name,
                                                    "arguments": ""
                                                }
                                            });
                                            yield Ok(Bytes::from(format!("event: response.output_item.added\ndata: {}\n\n", serde_json::to_string(&item).unwrap_or_default())));
                                        }
                                        "thinking" => {
                                            active_reasoning = Some((index, String::new()));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(delta) = event.get("delta") {
                                    match delta.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                                        "text_delta" => {
                                            if let (Some(text), Some((item_id, index, acc))) = (
                                                delta.get("text").and_then(|v| v.as_str()),
                                                active_text.as_mut(),
                                            ) {
                                                acc.push_str(text);
                                                let payload = json!({
                                                    "type": "response.output_text.delta",
                                                    "item_id": item_id,
                                                    "output_index": output_index,
                                                    "content_index": *index,
                                                    "delta": text
                                                });
                                                yield Ok(Bytes::from(format!("event: response.output_text.delta\ndata: {}\n\n", serde_json::to_string(&payload).unwrap_or_default())));
                                            }
                                        }
                                        "input_json_delta" => {
                                            if let (Some(partial), Some((item_id, _call_id, _name, arguments, tool_output_index))) = (
                                                delta.get("partial_json").and_then(|v| v.as_str()),
                                                active_tool.as_mut(),
                                            ) {
                                                arguments.push_str(partial);
                                                let payload = json!({
                                                    "type": "response.function_call_arguments.delta",
                                                    "item_id": item_id,
                                                    "output_index": *tool_output_index,
                                                    "delta": partial
                                                });
                                                yield Ok(Bytes::from(format!("event: response.function_call_arguments.delta\ndata: {}\n\n", serde_json::to_string(&payload).unwrap_or_default())));
                                            }
                                        }
                                        "thinking_delta" => {
                                            if let (Some(thinking), Some((_index, acc))) = (
                                                delta.get("thinking").and_then(|v| v.as_str()),
                                                active_reasoning.as_mut(),
                                            ) {
                                                acc.push_str(thinking);
                                                let payload = json!({
                                                    "type": "response.reasoning.delta",
                                                    "delta": thinking
                                                });
                                                yield Ok(Bytes::from(format!("event: response.reasoning.delta\ndata: {}\n\n", serde_json::to_string(&payload).unwrap_or_default())));
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "message_delta" => {
                                if let Some(delta) = event.get("delta") {
                                    stop_reason = delta.get("stop_reason").and_then(|v| v.as_str()).map(ToString::to_string);
                                }
                                if let Some(usage) = event.get("usage").and_then(|v| v.as_object()) {
                                    if let Some(map) = raw_usage.as_object_mut() {
                                        for (key, value) in usage {
                                            map.insert(key.clone(), value.clone());
                                        }
                                    }
                                }
                            }
                            "content_block_stop" => {
                                if let Some((item_id, index, text)) = active_text.take() {
                                    let done = json!({
                                        "type": "response.output_text.done",
                                        "item_id": item_id,
                                        "output_index": output_index,
                                        "content_index": index,
                                        "text": text
                                    });
                                    yield Ok(Bytes::from(format!("event: response.output_text.done\ndata: {}\n\n", serde_json::to_string(&done).unwrap_or_default())));

                                    let part_done = json!({
                                        "type": "response.content_part.done",
                                        "item_id": done["item_id"],
                                        "output_index": output_index,
                                        "content_index": index,
                                        "part": {
                                            "type": "output_text",
                                            "text": text,
                                            "annotations": []
                                        }
                                    });
                                    yield Ok(Bytes::from(format!("event: response.content_part.done\ndata: {}\n\n", serde_json::to_string(&part_done).unwrap_or_default())));

                                    final_content.push(json!({ "type": "text", "text": part_done["part"]["text"] }));
                                    let item_done = json!({
                                        "type": "response.output_item.done",
                                        "output_index": output_index,
                                        "item": {
                                            "id": done["item_id"],
                                            "type": "message",
                                            "status": "completed",
                                            "role": "assistant",
                                            "content": [{
                                                "type": "output_text",
                                                "text": done["text"],
                                                "annotations": []
                                            }]
                                        }
                                    });
                                    yield Ok(Bytes::from(format!("event: response.output_item.done\ndata: {}\n\n", serde_json::to_string(&item_done).unwrap_or_default())));
                                    output_index += 1;
                                } else if let Some((item_id, call_id, name, arguments, tool_output_index)) = active_tool.take() {
                                    let parsed = serde_json::from_str::<Value>(&arguments).unwrap_or_else(|_| json!({}));
                                    final_content.push(json!({
                                        "type": "tool_use",
                                        "id": call_id,
                                        "name": name,
                                        "input": parsed
                                    }));

                                    let args_done = json!({
                                        "type": "response.function_call_arguments.done",
                                        "item_id": item_id,
                                        "output_index": tool_output_index
                                    });
                                    yield Ok(Bytes::from(format!("event: response.function_call_arguments.done\ndata: {}\n\n", serde_json::to_string(&args_done).unwrap_or_default())));

                                    let item_done = json!({
                                        "type": "response.output_item.done",
                                        "output_index": tool_output_index,
                                        "item": {
                                            "id": item_id,
                                            "type": "function_call",
                                            "status": "completed",
                                            "call_id": final_content.last().and_then(|v| v.get("id")).cloned().unwrap_or(json!("")),
                                            "name": final_content.last().and_then(|v| v.get("name")).cloned().unwrap_or(json!("")),
                                            "arguments": serde_json::to_string(final_content.last().and_then(|v| v.get("input")).unwrap_or(&json!({}))).unwrap_or_else(|_| "{}".to_string())
                                        }
                                    });
                                    yield Ok(Bytes::from(format!("event: response.output_item.done\ndata: {}\n\n", serde_json::to_string(&item_done).unwrap_or_default())));
                                    output_index += 1;
                                } else if let Some((_index, thinking)) = active_reasoning.take() {
                                    if !thinking.is_empty() {
                                        final_content.push(json!({ "type": "thinking", "thinking": thinking }));
                                    }
                                    let payload = json!({
                                        "type": "response.reasoning.done"
                                    });
                                    yield Ok(Bytes::from(format!("event: response.reasoning.done\ndata: {}\n\n", serde_json::to_string(&payload).unwrap_or_default())));
                                }
                            }
                            "message_stop" => {
                                if let Some(recorder) = history_recorder.as_ref() {
                                    let history_content: Vec<Value> = final_content
                                        .iter()
                                        .filter(|block| {
                                            block.get("type").and_then(|value| value.as_str())
                                                != Some("thinking")
                                        })
                                        .cloned()
                                        .collect();
                                    if !message_id.is_empty() && !history_content.is_empty() {
                                        let mut history = request_messages.clone();
                                        history.push(json!({
                                            "role": "assistant",
                                            "content": history_content
                                        }));
                                        recorder(message_id.clone(), history);
                                    }
                                }

                                let final_response = anthropic_response_to_responses(json!({
                                    "id": message_id.clone(),
                                    "type": "message",
                                    "role": "assistant",
                                    "model": model.clone(),
                                    "content": final_content.clone(),
                                    "stop_reason": stop_reason.clone().unwrap_or_else(|| "end_turn".to_string()),
                                    "usage": raw_usage.clone()
                                })).unwrap_or_else(|_| json!({
                                    "id": message_id.clone(),
                                    "object": "response",
                                    "created_at": chrono::Utc::now().timestamp(),
                                    "model": model.clone(),
                                    "status": "completed",
                                    "output": [],
                                    "usage": build_responses_usage_from_anthropic(Some(&raw_usage))
                                }));

                                let completed = json!({
                                    "type": "response.completed",
                                    "response": final_response
                                });
                                yield Ok(Bytes::from(format!("event: response.completed\ndata: {}\n\n", serde_json::to_string(&completed).unwrap_or_default())));
                            }
                            _ => {}
                        }
                    }
                }
                Err(error) => {
                    yield Err(std::io::Error::other(error.to_string()));
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use futures::StreamExt;
    use std::collections::HashMap;

    #[test]
    fn test_map_responses_stop_reason_tool_use() {
        assert_eq!(
            map_responses_stop_reason(Some("completed"), true, None),
            Some("tool_use")
        );
        assert_eq!(
            map_responses_stop_reason(Some("completed"), false, None),
            Some("end_turn")
        );
        assert_eq!(
            map_responses_stop_reason(Some("incomplete"), false, Some("max_output_tokens")),
            Some("max_tokens")
        );
        assert_eq!(
            map_responses_stop_reason(Some("incomplete"), false, Some("content_filter")),
            Some("end_turn")
        );
    }

    #[test]
    fn test_response_object_from_event_with_wrapper() {
        let data = json!({
            "type": "response.created",
            "response": {
                "id": "resp_1",
                "model": "gpt-4o"
            }
        });
        let obj = response_object_from_event(&data);
        assert_eq!(obj["id"], "resp_1");
        assert_eq!(obj["model"], "gpt-4o");
    }

    #[tokio::test]
    async fn test_streaming_conversion_with_wrapped_response_events() {
        let input = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":12,\"output_tokens\":0}}}\n\n",
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"get_weather\"}}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{\\\"city\\\":\\\"Tokyo\\\"}\"}\n\n",
            "event: response.function_call_arguments.done\n",
            "data: {\"type\":\"response.function_call_arguments.done\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3}}}\n\n"
        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_anthropic_sse_stream_from_responses(upstream);
        let chunks: Vec<_> = converted.collect().await;

        let merged = chunks
            .into_iter()
            .map(|c| String::from_utf8_lossy(c.unwrap().as_ref()).to_string())
            .collect::<String>();

        assert!(merged.contains("\"type\":\"message_start\""));
        assert!(merged.contains("\"id\":\"resp_1\""));
        assert!(merged.contains("\"model\":\"gpt-4o\""));
        assert!(merged.contains("\"type\":\"tool_use\""));
        assert!(merged.contains("\"name\":\"get_weather\""));
        assert!(merged.contains("\"type\":\"input_json_delta\""));
        assert!(merged.contains("\"stop_reason\":\"tool_use\""));
        assert!(merged.contains("\"input_tokens\":12"));
        assert!(merged.contains("\"output_tokens\":3"));
        assert!(merged.contains("\"type\":\"message_stop\""));
    }

    #[tokio::test]
    async fn test_streaming_conversion_interleaved_tool_deltas_by_item_id() {
        let input = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_2\",\"model\":\"gpt-4o\"}}\n\n",
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"first_tool\"}}\n\n",
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"fc_2\",\"type\":\"function_call\",\"call_id\":\"call_2\",\"name\":\"second_tool\"}}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_2\",\"delta\":\"{\\\"b\\\":2}\"}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"a\\\":1}\"}\n\n",
            "event: response.function_call_arguments.done\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_1\"}\n\n",
            "event: response.function_call_arguments.done\n",
            "data: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_2\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":8,\"output_tokens\":4}}}\n\n"
        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_anthropic_sse_stream_from_responses(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let merged = chunks
            .into_iter()
            .map(|c| String::from_utf8_lossy(c.unwrap().as_ref()).to_string())
            .collect::<String>();

        let events: Vec<Value> = merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;
                serde_json::from_str::<Value>(data).ok()
            })
            .collect();

        let mut tool_index_by_call: HashMap<String, u64> = HashMap::new();
        for event in &events {
            if event.get("type").and_then(|v| v.as_str()) == Some("content_block_start") {
                let cb = event.get("content_block");
                if cb.and_then(|v| v.get("type")).and_then(|v| v.as_str()) == Some("tool_use") {
                    if let (Some(call_id), Some(index)) = (
                        cb.and_then(|v| v.get("id")).and_then(|v| v.as_str()),
                        event.get("index").and_then(|v| v.as_u64()),
                    ) {
                        tool_index_by_call.insert(call_id.to_string(), index);
                    }
                }
            }
        }

        let delta_indices: Vec<u64> = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_delta")
                    && event.pointer("/delta/type").and_then(|v| v.as_str())
                        == Some("input_json_delta")
            })
            .filter_map(|event| event.get("index").and_then(|v| v.as_u64()))
            .collect();

        assert_eq!(delta_indices.len(), 2);
        assert_eq!(delta_indices[0], *tool_index_by_call.get("call_2").unwrap());
        assert_eq!(delta_indices[1], *tool_index_by_call.get("call_1").unwrap());
        assert_ne!(
            tool_index_by_call.get("call_1"),
            tool_index_by_call.get("call_2")
        );
    }

    #[tokio::test]
    async fn test_streaming_reasoning_delta_emits_thinking_blocks() {
        let input = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_r\",\"model\":\"o3\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
            "event: response.reasoning.delta\n",
            "data: {\"type\":\"response.reasoning.delta\",\"delta\":\"Let me think...\"}\n\n",
            "event: response.reasoning.done\n",
            "data: {\"type\":\"response.reasoning.done\"}\n\n",
            "event: response.content_part.added\n",
            "data: {\"type\":\"response.content_part.added\",\"part\":{\"type\":\"output_text\",\"text\":\"\"},\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"42\",\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.content_part.done\n",
            "data: {\"type\":\"response.content_part.done\",\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":10}}}\n\n"
        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_anthropic_sse_stream_from_responses(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let merged = chunks
            .into_iter()
            .map(|c| String::from_utf8_lossy(c.unwrap().as_ref()).to_string())
            .collect::<String>();

        // Should contain thinking block start, thinking delta, and text content
        assert!(
            merged.contains("\"type\":\"thinking\""),
            "should emit thinking content_block_start"
        );
        assert!(
            merged.contains("\"type\":\"thinking_delta\""),
            "should emit thinking_delta"
        );
        assert!(
            merged.contains("\"thinking\":\"Let me think...\""),
            "should contain thinking text"
        );
        assert!(
            merged.contains("\"type\":\"text_delta\""),
            "should also emit text content"
        );
        assert!(
            merged.contains("\"text\":\"42\""),
            "should contain text delta"
        );
        assert!(merged.contains("\"stop_reason\":\"end_turn\""));
    }

    #[tokio::test]
    async fn test_streaming_text_parts_are_merged_into_one_text_block() {
        let input = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_merge\",\"model\":\"gpt-5.4\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
            "event: response.content_part.added\n",
            "data: {\"type\":\"response.content_part.added\",\"part\":{\"type\":\"output_text\",\"text\":\"\"},\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你\",\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.content_part.done\n",
            "data: {\"type\":\"response.content_part.done\",\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.content_part.added\n",
            "data: {\"type\":\"response.content_part.added\",\"part\":{\"type\":\"output_text\",\"text\":\"\"},\"output_index\":0,\"content_index\":1}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"好\",\"output_index\":0,\"content_index\":1}\n\n",
            "event: response.content_part.done\n",
            "data: {\"type\":\"response.content_part.done\",\"output_index\":0,\"content_index\":1}\n\n",
            "event: response.output_text.done\n",
            "data: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":1}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":2}}}\n\n"
        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_anthropic_sse_stream_from_responses(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let events: Vec<Value> = chunks
            .into_iter()
            .flat_map(|chunk| {
                let bytes = chunk.unwrap();
                let text = String::from_utf8_lossy(bytes.as_ref()).to_string();
                text.split("\n\n")
                    .filter_map(|block| {
                        block.lines().find_map(|line| {
                            strip_sse_field(line, "data")
                                .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let text_starts = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_start")
                    && event
                        .pointer("/content_block/type")
                        .and_then(|v| v.as_str())
                        == Some("text")
            })
            .count();
        let text_stops = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_stop")
            })
            .count();
        let text_deltas: Vec<String> = events
            .iter()
            .filter(|event| {
                event.get("type").and_then(|v| v.as_str()) == Some("content_block_delta")
                    && event.pointer("/delta/type").and_then(|v| v.as_str()) == Some("text_delta")
            })
            .filter_map(|event| {
                event
                    .pointer("/delta/text")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            })
            .collect();

        assert_eq!(text_starts, 1);
        assert_eq!(text_stops, 1);
        assert_eq!(text_deltas, vec!["你".to_string(), "好".to_string()]);
    }

    #[tokio::test]
    async fn test_streaming_responses_chinese_split_across_chunks_no_replacement_chars() {
        // Chinese text delta split across two TCP chunks.
        let full = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_cn\",\"model\":\"gpt-4o\",\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你好世界\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":4}}}\n\n"
        );
        let bytes = full.as_bytes();

        // Find "你" and split inside it
        let ni_start = bytes.windows(3).position(|w| w == "你".as_bytes()).unwrap();
        let split_point = ni_start + 2; // split after second byte of "你"

        let chunk1 = Bytes::from(bytes[..split_point].to_vec());
        let chunk2 = Bytes::from(bytes[split_point..].to_vec());

        let upstream = stream::iter(vec![
            Ok::<_, std::io::Error>(chunk1),
            Ok::<_, std::io::Error>(chunk2),
        ]);
        let converted = create_anthropic_sse_stream_from_responses(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let merged = chunks
            .into_iter()
            .map(|c| String::from_utf8_lossy(c.unwrap().as_ref()).to_string())
            .collect::<String>();

        assert!(
            merged.contains("你好世界"),
            "expected '你好世界' in output, got replacement chars (U+FFFD)"
        );
        assert!(
            !merged.contains('\u{FFFD}'),
            "output must not contain U+FFFD replacement characters"
        );
    }

    #[tokio::test]
    async fn test_streaming_anthropic_to_responses_emits_completed_usage() {
        let input = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_ds\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"deepseek-v3.2\",\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":60}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":12}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );

        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_responses_sse_stream_from_anthropic(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let merged = chunks
            .into_iter()
            .map(|c| String::from_utf8_lossy(c.unwrap().as_ref()).to_string())
            .collect::<String>();

        assert!(merged.contains("event: response.created"));
        assert!(merged.contains("event: response.output_text.delta"));
        assert!(merged.contains("event: response.completed"));
        assert!(merged.contains("\"cached_tokens\":60"));
        assert!(merged.contains("\"output_tokens\":12"));
        assert!(merged.contains("\"delta\":\"你好\""));
    }
}
