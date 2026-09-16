//! 临时调试模块：Kilo proxy 请求与流式响应聚合日志（写入 cc-switch.log）。
//!
//! 【临时代码】仅供开发调试使用；调试完成后删除本文件，
//! 以及 proxy/mod.rs 中 `mod kilo_debug` 与 proxy/kilo.rs 中对它的引用。
//!
//! 只观察、不转换：请求侧记录 Kilo->CC-Switch 与 CC-Switch->Provider 的 body；
//! 响应侧把一个 Run（一次请求 + 其完整流式响应）的所有 SSE delta 聚合成一条日志，
//! 在 Run 结束时一次性输出，而不是逐 event 打印。

use bytes::Bytes;
use futures::{Stream, StreamExt};

const LOG_PREFIX: &str = "[KILO_PROXY]";

/// 每条 Kilo 请求的唯一 ID；同一请求的所有日志复用该 ID。
pub(crate) fn new_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 记录 Kilo -> CC-Switch 的入站请求 body。日志侧把 `messages[].content` 等长字段
/// 替换为字面 `"***"` 以精简输出；实际转发 body 不受影响。
pub(crate) fn log_request_in(request_id: &str, body: &[u8]) {
    log_block(
        request_id,
        "",
        "REQ_IN_FROM_KILO",
        &truncated_request_body_for_log(body),
    );
}

/// 记录 CC-Switch -> Provider 的出站请求 body（传给 send_request 的最终 bytes）。
/// 同样做日志侧精简；实际转发 bytes 不受影响。
pub(crate) fn log_request_out(request_id: &str, body: &[u8]) {
    log_block(
        request_id,
        "",
        "REQ_OUT_TO_PROVIDER",
        &truncated_request_body_for_log(body),
    );
}

/// 将请求 body 解析为 JSON，把 `messages` 数组中每个元素的 `content` 字段、以及任意名为
/// `reasoning_content`、`tool_calls` 的字段整体替换为字面字符串 `"***"`，并把
/// `tools[].function.description/parameters` 替换为 `"***"`，其余字段（含顺序）保持不变后
/// 重新序列化为日志文本。
///
/// 仅用于日志精简：不脱敏、不省略，`***` 就是字面三个星号。解析失败则回退到原始字节文本，
/// 保证日志逻辑不干扰请求处理。
fn truncated_request_body_for_log(body: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return String::from_utf8_lossy(body).into_owned();
    };
    truncate_messages_content(&mut value);
    redact_named_fields(&mut value, &["reasoning_content", "tool_calls"]);
    truncate_tool_function_fields(&mut value, &["description", "parameters"]);
    serde_json::to_string(&value)
        .unwrap_or_else(|_| String::from_utf8_lossy(body).into_owned())
}

/// 把 `messages[].content`（存在时，字符串或数组均整体）替换为 `"***"`；其余字段不动。
fn truncate_messages_content(value: &mut serde_json::Value) {
    let Some(messages) = value.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return;
    };
    for message in messages.iter_mut() {
        if let Some(content) = message.get_mut("content") {
            *content = serde_json::Value::String("***".to_string());
        }
    }
}

/// 递归把任意名为给定列表中的字段值（顶层或嵌套）替换为 `"***"`；其余字段不动。
fn redact_named_fields(value: &mut serde_json::Value, names: &[&str]) {
    match value {
        serde_json::Value::Object(map) => {
            for name in names {
                if let Some(v) = map.get_mut(*name) {
                    *v = serde_json::Value::String("***".to_string());
                }
            }
            for child in map.values_mut() {
                redact_named_fields(child, names);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_named_fields(item, names);
            }
        }
        _ => {}
    }
}

/// 仅把 `tools[].function.<field>`（如 `description`、`parameters`）替换为 `"***"`：
/// `tools` 数组其他兄弟字段（如顶层 `tool_choice`）、以及 `function` 的其他兄弟字段
/// （如 `name`）均不受影响。
fn truncate_tool_function_fields(value: &mut serde_json::Value, fields: &[&str]) {
    let Some(tools) = value.get_mut("tools").and_then(|t| t.as_array_mut()) else {
        return;
    };
    for tool in tools.iter_mut() {
        let Some(function) = tool.get_mut("function") else {
            continue;
        };
        for field in fields {
            if let Some(v) = function.get_mut(*field) {
                *v = serde_json::Value::String("***".to_string());
            }
        }
    }
}

/// 仅把 `delta.tool_calls[].function.arguments` 替换为 `"***"`：tool_calls 内其他字段
/// （如 `id`、`type`、`function.name`）不受影响。
fn truncate_delta_tool_call_arguments(delta: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(tool_calls) = delta.get_mut("tool_calls").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for call in tool_calls.iter_mut() {
        if let Some(arguments) = call.get_mut("function").and_then(|f| f.get_mut("arguments")) {
            *arguments = serde_json::Value::String("***".to_string());
        }
    }
}

/// 将 `[BEGIN] / 原文 / [END]` 作为单条 log::info! 写出，保证整块不被其他线程日志打断。
/// 原文若以换行结尾（SSE event 总是如此），不再额外补换行，与示例格式一致。
fn log_block(request_id: &str, extra: &str, tag: &str, text: &str) {
    let begin = format!("{LOG_PREFIX}[request_id={request_id}]{extra}[{tag}][BEGIN]");
    let end = format!("{LOG_PREFIX}[request_id={request_id}]{extra}[{tag}][END]");
    if text.ends_with('\n') {
        log::info!("{begin}\n{text}{end}");
    } else {
        log::info!("{begin}\n{text}\n{end}");
    }
}

/// 在响应字节流外层包一个 Run 级聚合 tee。
///
/// - 仍按 bytes 层识别 SSE 事件终止符（`\n\n` / `\r\n\r\n`），把每个完整 event 的原始
///   bytes 原样透传，不改变顺序、内容或发送次数；
/// - 透传的同时把每个 event 的 `data:` JSON 合并进 Run 累加器，Run 结束时输出一条
///   RESP_IN_FROM_PROVIDER 聚合日志（只观察 Run，不逐 event 打印；出站与入站一致，不单独记录）；
/// - 连接正常结束或出错时保留上游语义，并透传残留 bytes；下游取消时经 Drop 输出
///   cancelled 状态的聚合日志。
pub(crate) fn sse_event_tee<S>(
    stream: S,
    request_id: String,
    response_headers: http::HeaderMap,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    async_stream::stream! {
        let mut stream = Box::pin(stream);
        let mut splitter = SseEventSplitter::new();
        let mut run = RunAccumulator::new(request_id, response_headers);

        loop {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    splitter.push_chunk(&chunk);
                    while let Some(event) = splitter.next_event() {
                        run.observe(&event);
                        yield Ok(event);
                    }
                }
                Some(Err(error)) => {
                    run.finalize(RunStatus::Error(error.to_string()));
                    if let Some(leftover) = splitter.take_remaining() {
                        yield Ok(leftover);
                    }
                    yield Err(error);
                    break;
                }
                None => {
                    let status = if run.saw_done { RunStatus::Done } else { RunStatus::Eof };
                    run.finalize(status);
                    if let Some(leftover) = splitter.take_remaining() {
                        yield Ok(leftover);
                    }
                    break;
                }
            }
        }
    }
}

/// 按 bytes 层把 chunk 流切成完整 SSE event（含终止空行）。
struct SseEventSplitter {
    buffer: Vec<u8>,
    /// buffer 中已确认不含终止符的前缀长度；下次搜索从此处往前最多 3 字节开始，
    /// 以覆盖跨 chunk 的终止符（最长 `\r\n\r\n` 为 4 字节）。
    scan_from: usize,
}

impl SseEventSplitter {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            scan_from: 0,
        }
    }

    fn push_chunk(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// 取出下一个完整 event 的原始 bytes（含终止空行）。没有完整 event 时推进
    /// scan_from 到尾部，使下一次搜索只覆盖可能跨 chunk 的终止符区域。
    fn next_event(&mut self) -> Option<Bytes> {
        match find_event_terminator(&self.buffer, self.scan_from) {
            Some((pos, len)) => {
                let end = pos + len;
                let event: Vec<u8> = self.buffer.drain(..end).collect();
                self.scan_from = 0;
                Some(Bytes::from(event))
            }
            None => {
                self.scan_from = self.buffer.len().saturating_sub(3);
                None
            }
        }
    }

    /// 连接结束时的残留 bytes（未以终止空行结束的不完整 event）。
    fn take_remaining(&mut self) -> Option<Bytes> {
        if self.buffer.is_empty() {
            return None;
        }
        self.scan_from = 0;
        Some(Bytes::from(std::mem::take(&mut self.buffer)))
    }
}

enum RunStatus {
    /// 收到 `data: [DONE]` 且流正常结束。
    Done,
    /// 流正常结束但未收到 `data: [DONE]`。
    Eof,
    /// 上游出错。
    Error(String),
    /// 下游取消（stream 被 drop）。
    Cancelled,
}

/// 把一个 Run 的流式 delta 合并成单个响应视图，并在结束时输出聚合日志。
struct RunAccumulator {
    request_id: String,
    response_headers: http::HeaderMap,
    delta: serde_json::Map<String, serde_json::Value>,
    delta_fields: Vec<String>,
    event_types: Vec<String>,
    usage: Option<serde_json::Value>,
    finish_reason: Option<serde_json::Value>,
    model: Option<String>,
    id: Option<String>,
    saw_done: bool,
    finalized: bool,
}

impl RunAccumulator {
    fn new(request_id: String, response_headers: http::HeaderMap) -> Self {
        Self {
            request_id,
            response_headers,
            delta: serde_json::Map::new(),
            delta_fields: Vec::new(),
            event_types: Vec::new(),
            usage: None,
            finish_reason: None,
            model: None,
            id: None,
            saw_done: false,
            finalized: false,
        }
    }

    /// 解析一个完整 SSE event（bytes），提取 `event:` 与 `data:` 行并合并。
    /// 非 JSON 的 `data:`（如 `[DONE]`、注释行）或解析失败仅跳过，不影响转发。
    fn observe(&mut self, event: &[u8]) {
        let Ok(text) = std::str::from_utf8(event) else {
            return;
        };
        let mut data_parts: Vec<&str> = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                let name = rest.strip_prefix(' ').unwrap_or(rest).trim();
                if !name.is_empty() {
                    self.event_types.push(name.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("data:") {
                data_parts.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        if data_parts.is_empty() {
            return;
        }
        let data = data_parts.join("\n");
        if data == "[DONE]" {
            self.saw_done = true;
            return;
        }
        let Ok(chunk) = serde_json::from_str::<serde_json::Value>(&data) else {
            return;
        };
        self.merge_chunk(&chunk);
    }

    fn merge_chunk(&mut self, chunk: &serde_json::Value) {
        if self.model.is_none() {
            if let Some(m) = chunk.get("model").and_then(|v| v.as_str()) {
                self.model = Some(m.to_string());
            }
        }
        if self.id.is_none() {
            if let Some(i) = chunk.get("id").and_then(|v| v.as_str()) {
                self.id = Some(i.to_string());
            }
        }
        if let Some(u) = chunk.get("usage") {
            self.usage = Some(u.clone());
        }
        let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
            return;
        };
        for choice in choices {
            if let Some(fr) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                self.finish_reason = Some(fr.clone());
            }
            if let Some(delta) = choice.get("delta").and_then(|d| d.as_object()) {
                for key in delta.keys() {
                    if !self.delta_fields.contains(key) {
                        self.delta_fields.push(key.clone());
                    }
                }
                merge_delta_into(&mut self.delta, delta);
            }
        }
    }

    fn finalize(&mut self, status: RunStatus) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        self.emit_log(&status);
    }

    fn emit_log(&self, status: &RunStatus) {
        let text = self.render(status);
        log_block(&self.request_id, "", "RESP_IN_FROM_PROVIDER", &text);
    }

    fn render(&self, status: &RunStatus) -> String {
        let mut obj = serde_json::Map::new();
        let status_str = match status {
            RunStatus::Done => "done",
            RunStatus::Eof => "eof",
            RunStatus::Error(_) => "error",
            RunStatus::Cancelled => "cancelled",
        };
        obj.insert(
            "status".to_string(),
            serde_json::Value::String(status_str.to_string()),
        );
        if let RunStatus::Error(e) = status {
            obj.insert("error".to_string(), serde_json::Value::String(e.clone()));
        }
        if let Some(model) = &self.model {
            obj.insert("model".to_string(), serde_json::Value::String(model.clone()));
        }
        if let Some(id) = &self.id {
            obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
        }
        obj.insert(
            "response_headers".to_string(),
            serde_json::Value::Object(render_response_headers(&self.response_headers)),
        );
        obj.insert(
            "delta_fields".to_string(),
            serde_json::Value::Array(
                self.delta_fields
                    .iter()
                    .map(|f| serde_json::Value::String(f.clone()))
                    .collect(),
            ),
        );
        obj.insert(
            "event_types".to_string(),
            serde_json::Value::Array(
                self.event_types
                    .iter()
                    .map(|f| serde_json::Value::String(f.clone()))
                    .collect(),
            ),
        );
        // 日志侧精简 delta：reasoning_content 与 tool_calls[].function.arguments 替换为
        // 字面 "***"；累加器仍保留原文。
        let mut delta = self.delta.clone();
        if let Some(v) = delta.get_mut("reasoning_content") {
            *v = serde_json::Value::String("***".to_string());
        }
        truncate_delta_tool_call_arguments(&mut delta);
        obj.insert("delta".to_string(), serde_json::Value::Object(delta));
        if let Some(u) = &self.usage {
            obj.insert("usage".to_string(), u.clone());
        }
        if let Some(fr) = &self.finish_reason {
            obj.insert("finish_reason".to_string(), fr.clone());
        }
        serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default()
    }
}

impl Drop for RunAccumulator {
    fn drop(&mut self) {
        if !self.finalized {
            self.finalized = true;
            self.emit_log(&RunStatus::Cancelled);
        }
    }
}

/// 把上游响应头渲染成 `name -> value` 的 JSON 对象。同名多值用 `, ` 拼接；
/// 无法按 UTF-8 解析的值记为 `<binary>`。
fn render_response_headers(headers: &http::HeaderMap) -> serde_json::Map<String, serde_json::Value> {
    let mut map: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    for (name, value) in headers {
        let name = name.as_str().to_string();
        let value = value
            .to_str()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "<binary>".to_string());
        match map.get_mut(&name) {
            Some(serde_json::Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => {
                map.insert(name, serde_json::Value::String(value));
            }
        }
    }
    map
}

/// 把一次 `choices[].delta` 合并进累加 delta：字符串字段拼接，`tool_calls` 按 index 合并，
/// 其余字段后值覆盖。
fn merge_delta_into(
    acc: &mut serde_json::Map<String, serde_json::Value>,
    delta: &serde_json::Map<String, serde_json::Value>,
) {
    for (key, val) in delta {
        if key == "tool_calls" {
            merge_tool_calls_into(acc, val);
            continue;
        }
        match acc.get_mut(key) {
            Some(existing) => merge_value(existing, val),
            None => {
                acc.insert(key.clone(), val.clone());
            }
        }
    }
}

fn merge_value(target: &mut serde_json::Value, incoming: &serde_json::Value) {
    match (target, incoming) {
        (serde_json::Value::Object(t), serde_json::Value::Object(i)) => {
            for (k, v) in i {
                match t.get_mut(k) {
                    Some(e) => merge_value(e, v),
                    None => {
                        t.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (serde_json::Value::String(t), serde_json::Value::String(i)) => t.push_str(i),
        (t, i) => *t = i.clone(),
    }
}

fn merge_tool_calls_into(
    acc: &mut serde_json::Map<String, serde_json::Value>,
    incoming: &serde_json::Value,
) {
    let serde_json::Value::Array(items) = incoming else {
        return;
    };
    let slot = acc
        .entry("tool_calls".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let serde_json::Value::Array(arr) = slot else {
        return;
    };
    for item in items {
        let serde_json::Value::Object(obj) = item else {
            continue;
        };
        let index = obj
            .get("index")
            .and_then(|v| v.as_u64())
            .unwrap_or(arr.len() as u64) as usize;
        while arr.len() <= index {
            arr.push(serde_json::Value::Object(serde_json::Map::new()));
        }
        let serde_json::Value::Object(target) = &mut arr[index] else {
            continue;
        };
        for (k, v) in obj {
            if k == "index" {
                continue;
            }
            match target.get_mut(k) {
                Some(e) => merge_value(e, v),
                None => {
                    target.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// 在 `buffer[scan_from..]` 中寻找最早的 SSE 事件终止符（`\n\n` 或 `\r\n\r\n`），
/// 返回 (终止符起始绝对位置, 终止符长度)。
fn find_event_terminator(buffer: &[u8], scan_from: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for (pattern, len) in [(b"\n\n" as &[u8], 2usize), (b"\r\n\r\n" as &[u8], 4usize)] {
        if let Some(pos) = find_subslice(buffer, scan_from, pattern) {
            if best.is_none_or(|(best_pos, _)| pos < best_pos) {
                best = Some((pos, len));
            }
        }
    }
    best
}

fn find_subslice(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let last_start = haystack.len() - needle.len();
    let start = from.min(last_start);
    (start..=last_start).find(|&i| &haystack[i..i + needle.len()] == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(items: Vec<&[u8]>) -> Vec<Result<Bytes, std::io::Error>> {
        items
            .into_iter()
            .map(|b| Ok(Bytes::copy_from_slice(b)))
            .collect()
    }

    #[test]
    fn truncates_messages_content_only_and_keeps_other_fields() {
        let body = br#"{"model":"x","stream":true,"reasoning":true,"messages":[{"role":"system","content":"very long system prompt"},{"role":"user","content":"hello world"},{"role":"assistant","tool_calls":[{"id":"c","function":{"name":"f","arguments":"{}"}}]}]}"#;
        let out = truncated_request_body_for_log(body);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "x");
        assert_eq!(v["stream"], true);
        assert_eq!(v["reasoning"], true);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][0]["content"], "***");
        assert_eq!(v["messages"][1]["content"], "***");
        // assistant 消息无 content 字段，不应新增
        assert!(v["messages"][2].get("content").is_none());
        assert_eq!(v["messages"][2]["tool_calls"], "***");
    }

    #[test]
    fn truncates_array_content_to_literal_stars() {
        let body = br#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:..."}}]}]}"#;
        let out = truncated_request_body_for_log(body);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["messages"][0]["content"], "***");
    }

    #[test]
    fn truncation_preserves_field_order() {
        let body = br#"{"z":1,"a":2,"messages":[]}"#;
        let out = truncated_request_body_for_log(body);
        let z = out.find("\"z\"").unwrap();
        let a = out.find("\"a\"").unwrap();
        let m = out.find("\"messages\"").unwrap();
        assert!(z < a && a < m);
    }

    #[test]
    fn truncation_falls_back_to_raw_on_invalid_json() {
        let body = b"not json at all";
        assert_eq!(truncated_request_body_for_log(body), "not json at all");
    }

    #[test]
    fn truncates_reasoning_content_at_top_level_and_in_messages() {
        let body = br#"{"model":"x","reasoning_content":"long chain of thought","messages":[{"role":"assistant","reasoning_content":"nested reasoning","content":"hi"}]}"#;
        let out = truncated_request_body_for_log(body);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["reasoning_content"], "***");
        assert_eq!(v["messages"][0]["reasoning_content"], "***");
        assert_eq!(v["messages"][0]["content"], "***");
    }

    #[test]
    fn truncates_tool_calls_field() {
        let body = br#"{"messages":[{"role":"assistant","content":"hi","tool_calls":[{"id":"c","function":{"name":"f","arguments":"{\"q\":1}"}}]}]}"#;
        let out = truncated_request_body_for_log(body);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["messages"][0]["content"], "***");
        assert_eq!(v["messages"][0]["tool_calls"], "***");
    }

    #[test]
    fn truncates_only_tools_function_description_and_parameters() {
        let body = br#"{"model":"x","tool_choice":"auto","tools":[{"type":"function","function":{"name":"get_weather","description":"Get the current weather in a given location","parameters":{"type":"object","properties":{"location":{"type":"string"}}}}},{"type":"function","function":{"name":"other","description":"another long description","parameters":{"type":"object"}}}]}"#;
        let out = truncated_request_body_for_log(body);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["tool_choice"], "auto");
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(v["tools"][0]["function"]["description"], "***");
        assert_eq!(v["tools"][0]["function"]["parameters"], "***");
        assert_eq!(v["tools"][1]["function"]["description"], "***");
        assert_eq!(v["tools"][1]["function"]["parameters"], "***");
    }

    #[test]
    fn finds_lf_and_crlf_terminators_and_takes_earliest() {
        assert_eq!(find_event_terminator(b"a\n\nb", 0), Some((1, 2)));
        assert_eq!(find_event_terminator(b"a\r\n\r\nb", 0), Some((1, 4)));
        assert_eq!(find_event_terminator(b"a\n\nb\r\n\r\nc", 0), Some((1, 2)));
        assert_eq!(find_event_terminator(b"a\r\n\r\nb\n\nc", 0), Some((1, 4)));
        assert_eq!(find_event_terminator(b"no terminator", 0), None);
    }

    #[test]
    fn splitter_yields_complete_events_in_order() {
        let mut splitter = SseEventSplitter::new();
        splitter.push_chunk(b"event: x\ndata: 1\n\ndata: 2\n\ndata: [DONE]\n\n");
        let mut events = Vec::new();
        while let Some(e) = splitter.next_event() {
            events.push(e);
        }
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], b"event: x\ndata: 1\n\n".as_slice());
        assert_eq!(events[1], b"data: 2\n\n".as_slice());
        assert_eq!(events[2], b"data: [DONE]\n\n".as_slice());
    }

    #[test]
    fn event_split_across_chunks() {
        let mut splitter = SseEventSplitter::new();
        splitter.push_chunk(b"data: 1\n");
        assert!(splitter.next_event().is_none());
        splitter.push_chunk(b"\ndata: 2\n\n");
        let mut events = Vec::new();
        while let Some(e) = splitter.next_event() {
            events.push(e);
        }
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], b"data: 1\n\n".as_slice());
        assert_eq!(events[1], b"data: 2\n\n".as_slice());
    }

    #[test]
    fn utf8_char_split_across_chunks_preserves_bytes() {
        let mut splitter = SseEventSplitter::new();
        let full = "data: {\"t\":\"你好\"}\n\n".as_bytes();
        let mid = full.len() / 2;
        splitter.push_chunk(&full[..mid]);
        splitter.push_chunk(&full[mid..]);
        let event = splitter.next_event().unwrap();
        assert_eq!(event, full);
    }

    #[tokio::test]
    async fn tee_passthrough_is_byte_identical() {
        let input: Vec<u8> = b"data: {\"a\":1}\n\ndata: [DONE]\n\n".to_vec();
        let stream = futures::stream::iter(chunks(vec![&input[..]]));
        let tee = sse_event_tee(stream, "id".to_string(), http::HeaderMap::new());
        futures::pin_mut!(tee);
        let mut out = Vec::new();
        while let Some(Ok(b)) = tee.next().await {
            out.extend_from_slice(&b);
        }
        assert_eq!(out, input);
    }

    #[tokio::test]
    async fn tee_propagates_error_after_flushing_partial() {
        let stream = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"data: partial")),
            Err(std::io::Error::new(std::io::ErrorKind::Other, "boom")),
        ]);
        let tee = sse_event_tee(stream, "id".to_string(), http::HeaderMap::new());
        futures::pin_mut!(tee);
        let mut out = Vec::new();
        let mut errored = false;
        while let Some(item) = tee.next().await {
            match item {
                Ok(b) => out.extend_from_slice(&b),
                Err(_) => errored = true,
            }
        }
        assert_eq!(out, b"data: partial");
        assert!(errored);
    }

    #[tokio::test]
    async fn tee_forwards_trailing_partial_on_clean_end() {
        let stream = futures::stream::iter(chunks(vec![b"data: a\n\nb: tail"]));
        let tee = sse_event_tee(stream, "id".to_string(), http::HeaderMap::new());
        futures::pin_mut!(tee);
        let mut out = Vec::new();
        while let Some(Ok(b)) = tee.next().await {
            out.extend_from_slice(&b);
        }
        assert_eq!(out, b"data: a\n\nb: tail");
    }

    #[test]
    fn run_accumulator_merges_delta_strings_and_detects_done() {
        let mut run = RunAccumulator::new("t".to_string(), http::HeaderMap::new());
        run.observe(
            br#"data: {"id":"r1","model":"m1","choices":[{"delta":{"role":"assistant","reasoning_content":"hello "}}]}

"#,
        );
        run.observe(
            br#"data: {"choices":[{"delta":{"reasoning_content":"world"}}]}

"#,
        );
        run.observe(
            br#"data: {"choices":[{"delta":{"content":"hi"}}]}

"#,
        );
        run.observe(
            br#"data: [DONE]

"#,
        );
        assert!(run.saw_done);
        assert_eq!(run.id.as_deref(), Some("r1"));
        assert_eq!(run.model.as_deref(), Some("m1"));
        assert_eq!(run.delta["role"].as_str(), Some("assistant"));
        assert_eq!(run.delta["reasoning_content"].as_str(), Some("hello world"));
        assert_eq!(run.delta["content"].as_str(), Some("hi"));
        assert!(run.delta_fields.contains(&"reasoning_content".to_string()));
        assert!(run.delta_fields.contains(&"content".to_string()));
        assert!(run.delta_fields.contains(&"role".to_string()));
    }

    #[test]
    fn run_accumulator_merges_tool_calls_by_index() {
        let mut run = RunAccumulator::new("t".to_string(), http::HeaderMap::new());
        run.observe(
            br#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"get_w","arguments":"{\"a\":"}}]}}]}

"#,
        );
        run.observe(
            br#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}

"#,
        );
        let tc = run.delta["tool_calls"].as_array().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0]["id"].as_str(), Some("c1"));
        assert_eq!(tc[0]["function"]["name"].as_str(), Some("get_w"));
        assert_eq!(tc[0]["function"]["arguments"].as_str(), Some("{\"a\":1}"));
    }

    #[test]
    fn run_accumulator_records_event_types_and_usage() {
        let mut run = RunAccumulator::new("t".to_string(), http::HeaderMap::new());
        run.observe(
            br#"event: message_start
data: {"choices":[{"delta":{"role":"assistant"}}]}

"#,
        );
        run.observe(
            br#"data: {"choices":[],"usage":{"completion_tokens_details":{"reasoning_tokens":209}}}

"#,
        );
        assert_eq!(run.event_types, vec!["message_start".to_string()]);
        assert_eq!(
            run.usage.as_ref().unwrap()["completion_tokens_details"]["reasoning_tokens"],
            209
        );
    }

    #[test]
    fn run_accumulator_renders_summary() {
        let mut run = RunAccumulator::new("t".to_string(), http::HeaderMap::new());
        run.observe(
            br#"data: {"model":"m1","choices":[{"delta":{"reasoning_content":"abc"}}],"usage":{"completion_tokens_details":{"reasoning_tokens":3}}}

"#,
        );
        run.observe(
            br#"data: [DONE]

"#,
        );
        let text = run.render(&RunStatus::Done);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["status"], "done");
        assert_eq!(v["model"], "m1");
        assert_eq!(v["delta"]["reasoning_content"], "***");
        assert_eq!(v["delta_fields"][0], "reasoning_content");
        assert_eq!(v["usage"]["completion_tokens_details"]["reasoning_tokens"], 3);
    }

    #[test]
    fn run_accumulator_renders_truncated_tool_call_arguments() {
        let mut run = RunAccumulator::new("t".to_string(), http::HeaderMap::new());
        run.observe(
            br#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"get_w","arguments":"{\"a\":1}"}}]}}]}

"#,
        );
        run.observe(
            br#"data: [DONE]

"#,
        );
        let text = run.render(&RunStatus::Done);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let tc = v["delta"]["tool_calls"].as_array().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0]["id"].as_str(), Some("c1"));
        assert_eq!(tc[0]["function"]["name"].as_str(), Some("get_w"));
        assert_eq!(tc[0]["function"]["arguments"].as_str(), Some("***"));
    }
}
