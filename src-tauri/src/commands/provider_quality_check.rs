//! 鹈鹕测试：独立的流式 Chat Completions 测试，不经过代理转发或熔断器。
//!
//! 我们自己的功能改动都放在这个文件里；`stream_check` 只 re-export 命令，
//! 上游对该文件的重写不会冲掉这块。

use crate::app_config::AppType;
use crate::error::AppError;
use crate::store::AppState;
use tauri::State;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderChatTestResult {
    http_status: u16,
    response_time_ms: u64,
}

#[tauri::command]
pub async fn test_provider_chat(
    state: State<'_, AppState>,
    app: String,
    provider_id: String,
    on_event: tauri::ipc::Channel<String>,
) -> Result<ProviderChatTestResult, AppError> {
    use futures::StreamExt;
    use http::{header, HeaderMap, HeaderName, HeaderValue, Method, Uri};
    use serde_json::json;
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    let app_type = AppType::from_str(&app)?;

    let providers = state.db.get_all_providers(&app_type)?;
    let provider = providers
        .get(&provider_id)
        .ok_or_else(|| AppError::Message(format!("供应商 {provider_id} 不存在")))?;
    let settings = &provider.settings_config;
    let nonempty = |value: Option<&serde_json::Value>| {
        value
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let adapter = crate::proxy::providers::get_adapter(&app_type);
    let base_url = nonempty(settings.pointer("/options/baseURL"))
        .or_else(|| nonempty(settings.get("base_url")))
        .or_else(|| nonempty(settings.get("baseURL")))
        .or_else(|| {
            adapter
                .as_ref()
                .and_then(|a| a.extract_base_url(provider).ok())
        })
        .ok_or_else(|| AppError::Message("供应商缺少 baseURL".into()))?;
    let api_key = nonempty(settings.pointer("/options/apiKey"))
        .or_else(|| nonempty(settings.get("api_key")))
        .or_else(|| nonempty(settings.get("apiKey")))
        .or_else(|| {
            adapter
                .as_ref()
                .and_then(|a| a.extract_auth(provider))
                .map(|a| a.api_key)
        })
        .ok_or_else(|| AppError::Message("供应商缺少 API Key".into()))?;
    let model = nonempty(settings.get("model"))
        .or_else(|| {
            settings
                .get("models")
                .and_then(|v| v.as_object())
                .filter(|models| models.len() == 1)
                .and_then(|models| models.keys().next())
                .cloned()
        })
        .or_else(|| nonempty(settings.pointer("/env/ANTHROPIC_MODEL")))
        .or_else(|| {
            settings
                .get("config")
                .and_then(|v| v.as_str())
                .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
                .and_then(|doc| doc.get("model").and_then(|v| v.as_str()).map(str::to_owned))
        })
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| AppError::Message("供应商缺少明确的模型配置".into()))?;
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    };
    let uri: Uri = url
        .parse()
        .map_err(|e| AppError::Message(format!("供应商地址无效: {e}")))?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return Err(AppError::Message("供应商地址必须使用 http 或 https".into()));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| AppError::Message("供应商地址缺少主机".into()))?;
    let mut headers = HeaderMap::new();
    if let Some(custom) = settings
        .pointer("/options/headers")
        .and_then(|v| v.as_object())
    {
        for (name, value) in custom {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| AppError::Message("自定义请求头名称无效".into()))?;
            let value = value
                .as_str()
                .ok_or_else(|| AppError::Message("自定义请求头必须是字符串".into()))?;
            headers.insert(
                name,
                HeaderValue::from_str(value)
                    .map_err(|_| AppError::Message("自定义请求头值无效".into()))?,
            );
        }
    }
    headers.insert(
        header::HOST,
        HeaderValue::from_str(authority.as_str())
            .map_err(|_| AppError::Message("供应商主机无效".into()))?,
    );
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| AppError::Message("API Key 不是有效的请求头值".into()))?,
    );
    let body = serde_json::to_vec(&json!({
        "model": model,
        "messages": [{"role": "user", "content": "创建一个 SVG，内容绘制一个鹈鹕骑自行车的2D动画。\n仅输出 SVG 代码。"}],
        "stream": true
    }))
    .map_err(|e| AppError::Message(e.to_string()))?;
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string())
            .map_err(|e| AppError::Message(e.to_string()))?,
    );
    let proxy = nonempty(settings.pointer("/options/proxy"));
    let started = Instant::now();
    // send_request 的 timeout 仅覆盖响应头；读取响应体时单独按每次收包计时。
    // 提供原始头信息以沿用支持 provider proxy 的底层发送路径。
    let mut extensions = http::Extensions::new();
    extensions.insert(crate::proxy::hyper_client::OriginalHeaderCases {
        cases: headers
            .keys()
            .map(|name| (name.as_str().to_string(), name.as_str().as_bytes().to_vec()))
            .collect(),
    });
    let response = crate::proxy::hyper_client::send_request(
        uri,
        "Provider chat test",
        Method::POST,
        headers,
        extensions,
        body,
        Duration::from_secs(30),
        proxy.as_deref(),
    )
    .await
    .map_err(|e| AppError::Message(e.to_string()))?;
    let http_status = response.status().as_u16();
    let is_sse = response.is_sse();
    on_event
        .send(format!("HTTP {http_status}\n"))
        .map_err(|e| AppError::Message(e.to_string()))?;
    let mut stream = Box::pin(response.bytes_stream());
    let mut pending = Vec::new();
    let mut done = false;
    while !done {
        let next = tokio::time::timeout(Duration::from_secs(30), stream.next())
            .await
            .map_err(|_| AppError::Message("连续 30 秒未收到数据，鹈鹕测试已中断".into()))?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| AppError::Message(e.to_string()))?;
        pending.extend_from_slice(&chunk);
        if pending.len() > 1024 * 1024 {
            return Err(AppError::Message("单条流式事件超过 1 MB".into()));
        }
        if is_sse {
            while let Some((end, delimiter_len)) = sse_event_boundary(&pending) {
                let event = String::from_utf8_lossy(&pending[..end]).into_owned();
                pending.drain(..end + delimiter_len);
                done = event.lines().any(|line| {
                    line.strip_prefix("data:")
                        .is_some_and(|data| data.trim() == "[DONE]")
                });
                on_event
                    .send(format!("{event}\n\n"))
                    .map_err(|e| AppError::Message(e.to_string()))?;
                if done {
                    break;
                }
            }
        }
    }
    // 非 SSE 错误正文、或最后未带空行的事件仍保留显示。
    if !done && !pending.is_empty() {
        on_event
            .send(String::from_utf8_lossy(&pending).into_owned())
            .map_err(|e| AppError::Message(e.to_string()))?;
    }
    Ok(ProviderChatTestResult {
        http_status,
        response_time_ms: started.elapsed().as_millis() as u64,
    })
}

fn sse_event_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|i| (i, 2))
        .into_iter()
        .chain(
            bytes
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| (i, 4)),
        )
        .min_by_key(|(index, _)| *index)
}

#[cfg(test)]
mod chat_stream_tests {
    use super::sse_event_boundary;

    #[test]
    fn boundaries_support_split_and_multiple_sse_events() {
        assert_eq!(sse_event_boundary(b"data: {}\r\n\r"), None);
        assert_eq!(sse_event_boundary(b"data: {}\r\n\r\n"), Some((8, 4)));
        assert_eq!(
            sse_event_boundary(b"data: {}\n\ndata: [DONE]\n\n"),
            Some((8, 2))
        );
        assert_eq!(
            sse_event_boundary(b"data: {}\r\n\r\ndata: {}\n\n"),
            Some((8, 4))
        );
    }
}
