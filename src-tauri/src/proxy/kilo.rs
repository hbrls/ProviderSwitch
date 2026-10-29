//! 独立的 Kilo OpenAI-compatible Chat Completions proxy。
//!
//! Kilo 使用自己的路由和转发链路；这里只复用底层 HTTP client、数据库和熔断器
//! 基础设施，不进入 Codex 的 handler、adapter 或请求转换流程。

use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::time::Duration;

use crate::{
    provider::Provider,
    proxy::{
        content_encoding::{decompress_body, get_content_encoding, is_supported_content_encoding},
        handler_config::KILO_PARSER_CONFIG,
        hyper_client::{send_request, MAX_RESPONSE_BODY_BYTES},
        response_processor::{process_response, usage_logging_enabled, ResponseContext},
        server::ProxyState,
        session::extract_session_id,
        usage::logger::UsageLogger,
        ProxyError,
    },
};

const APP_TYPE: &str = "kilo";
const TAG: &str = "Kilo";
const MAX_REQUEST_BODY_BYTES: usize = 200 * 1024 * 1024;

/// Kilo 旁路统计上下文：在一次请求生命周期内捕获实际 Provider、CC-Switch
/// 配置的出站模型、session ID 和流式标记，供成功/失败路径统一写入
/// `app_type = kilo` 的代理统计记录。统计写入失败不影响业务响应。
#[derive(Debug, Clone)]
struct KiloUsageContext {
    provider_id: String,
    outbound_model: String,
    session_id: String,
    is_stream: bool,
}

#[derive(Debug, Clone)]
pub struct KiloProviderConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub headers: HeaderMap,
    pub thinking: Value,
    pub reasoning_effort: Value,
}

/// Validate the persisted Kilo provider shape. This is also used by ProviderService
/// before a Kilo provider is saved.
pub fn validate_provider_settings(settings: &Value) -> Result<(), String> {
    let object = settings
        .as_object()
        .ok_or_else(|| "Kilo 配置必须是 JSON 对象".to_string())?;
    let options = object
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| "Kilo 配置缺少 options".to_string())?;
    let base_url = options
        .get("baseURL")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Kilo 配置缺少 options.baseURL".to_string())?;
    let parsed = url::Url::parse(base_url).map_err(|e| format!("Kilo baseURL 无效: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Kilo baseURL 必须使用 http 或 https".to_string());
    }
    let api_key = options
        .get("apiKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Kilo 配置缺少 options.apiKey".to_string())?;
    if api_key.contains(['\r', '\n']) {
        return Err("Kilo apiKey 不能包含换行符".to_string());
    }
    let models = object
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| "Kilo 配置缺少 models".to_string())?;
    if models.len() != 1
        || models
            .keys()
            .next()
            .is_none_or(|model| model.trim().is_empty())
    {
        return Err("Kilo 配置必须包含且仅包含一个模型".to_string());
    }

    let thinking = object
        .get("thinking")
        .and_then(Value::as_object)
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Kilo 配置缺少 thinking.type".to_string())?;
    let reasoning_effort = object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Kilo 配置缺少 reasoning_effort".to_string())?;

    let normalized_url = base_url.trim_end_matches('/').to_ascii_lowercase();
    if normalized_url.ends_with("/kilo/v1") || normalized_url.ends_with("/kilo/v1/chat/completions")
    {
        return Err("Kilo baseURL 不能指向当前 Kilo proxy 路由".to_string());
    }

    if let Some(headers) = options.get("headers") {
        let headers = headers
            .as_object()
            .ok_or_else(|| "Kilo headers 必须是 JSON 对象".to_string())?;
        for (name, value) in headers {
            let parsed_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("Kilo header 名称无效: {e}"))?;
            let lower = parsed_name.as_str().to_ascii_lowercase();
            if is_forbidden_header(&lower) {
                return Err(format!("Kilo header 不允许覆盖 {name}"));
            }
            let value = value
                .as_str()
                .ok_or_else(|| format!("Kilo header {name} 必须是字符串"))?;
            HeaderValue::from_str(value)
                .map_err(|e| format!("Kilo header {name} 的值无效: {e}"))?;
        }
    }

    // Keep these bindings visible to make the required fields explicit to readers and
    // avoid accidentally weakening validation during later edits.
    let _ = (thinking, reasoning_effort);
    Ok(())
}

pub fn parse_provider(provider: &Provider) -> Result<KiloProviderConfig, ProxyError> {
    validate_provider_settings(&provider.settings_config).map_err(ProxyError::ConfigError)?;
    let object = provider
        .settings_config
        .as_object()
        .ok_or_else(|| ProxyError::ConfigError("Kilo 配置必须是 JSON 对象".to_string()))?;
    let mut headers = HeaderMap::new();
    let options = object["options"].as_object().unwrap();
    if let Some(values) = options.get("headers").and_then(Value::as_object) {
        for (name, value) in values {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| ProxyError::ConfigError(format!("Kilo header 名称无效: {e}")))?;
            let value = HeaderValue::from_str(value.as_str().unwrap_or_default())
                .map_err(|e| ProxyError::ConfigError(format!("Kilo header 值无效: {e}")))?;
            headers.insert(name, value);
        }
    }
    Ok(KiloProviderConfig {
        base_url: options
            .get("baseURL")
            .and_then(Value::as_str)
            .unwrap()
            .trim()
            .trim_end_matches('/')
            .to_string(),
        api_key: options["apiKey"].as_str().unwrap().trim().to_string(),
        model: object["models"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .trim()
            .to_string(),
        headers,
        thinking: object["thinking"].clone(),
        reasoning_effort: object["reasoning_effort"].clone(),
    })
}

fn apply_provider_request_overrides(body: &mut Value, provider: &KiloProviderConfig) {
    let Some(object) = body.as_object_mut() else {
        return;
    };

    object.insert("thinking".to_string(), provider.thinking.clone());
    object.insert(
        "reasoning_effort".to_string(),
        provider.reasoning_effort.clone(),
    );
}

fn is_forbidden_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "x-api-key"
            | "x-goog-api-key"
            | "host"
            | "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

pub async fn handle_chat_completions(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<Response, Response> {
    let mode = crate::mode::current::mode_state(&crate::app_config::AppType::Kilo);
    if !mode.is_proxy() || !mode.attached {
        return Err(kilo_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "kilo_proxy_disabled",
            "Kilo proxy channel is disabled",
        ));
    }
    let config = state
        .db
        .get_proxy_config_for_app(APP_TYPE)
        .await
        .map_err(|e| {
            kilo_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "proxy_config_error",
                e.to_string(),
            )
        })?;
    {
        let mut status = state.status.write().await;
        status.total_requests = status.total_requests.saturating_add(1);
        status.last_request_at = Some(chrono::Utc::now().to_rfc3339());
    }

    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    let extensions = parts.extensions;
    let mut body_bytes = body
        .collect()
        .await
        .map_err(|e| kilo_error(StatusCode::BAD_REQUEST, "invalid_request", e.to_string()))?
        .to_bytes();
    if body_bytes.len() > MAX_REQUEST_BODY_BYTES {
        return Err(kilo_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_request",
            "Request body is too large",
        ));
    }
    body_bytes = decode_request_body(&mut headers, body_bytes)?;
    let mut body: Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        kilo_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("Invalid JSON request body: {e}"),
        )
    })?;
    if !body.is_object() {
        return Err(kilo_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Request body must be a JSON object",
        ));
    }

    let app_type = crate::app_config::AppType::Kilo;
    let current_id = crate::mode::current::provider_for(
        &state.db,
        &app_type,
        crate::mode::current::Purpose::InUse,
    )
    .map_err(|e| {
        kilo_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "provider_error",
            e.to_string(),
        )
    })?
    .ok_or_else(|| {
        kilo_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_provider_configured",
            "No current Kilo provider is configured",
        )
    })?;
    let provider = state
        .db
        .get_provider_by_id(&current_id, APP_TYPE)
        .map_err(|e| {
            kilo_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "provider_error",
                e.to_string(),
            )
        })?
        .ok_or_else(|| {
            kilo_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "no_provider_configured",
                "The current Kilo provider does not exist",
            )
        })?;

    let started = std::time::Instant::now();
    let provider_config = match parse_provider(&provider) {
        Ok(value) => value,
        Err(error) => {
            let _ = state
                .provider_router
                .record_result(
                    &provider.id,
                    APP_TYPE,
                    false,
                    false,
                    Some(error.to_string()),
                )
                .await;
            return Err(kilo_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_provider_config",
                error.to_string(),
            ));
        }
    };
    // Capture server-owned routing context before the body is rewritten.
    // CC-Switch's configured model is authoritative for Kilo; Kilo does not
    // provide a meaningful request-side model.
    let outbound_model = provider_config.model.clone();
    // Kilo outbound requests are always forced to streaming mode by CC-Switch.
    let is_stream = true;
    let session_id = extract_session_id(&headers, &body, APP_TYPE).session_id;
    // Routing is entirely server-owned. Ignore any client-supplied model
    // or provider selector fields instead of forwarding them upstream.
    if let Some(object) = body.as_object_mut() {
        object.remove("provider");
        object.remove("provider_id");
        object.remove("providerId");
        object.insert(
            "model".to_string(),
            Value::String(provider_config.model.clone()),
        );
        object.insert("stream".to_string(), Value::Bool(true));
    }
    apply_provider_request_overrides(&mut body, &provider_config);
    // Always request usage in the final streaming chunk.
    if let Some(object) = body.as_object_mut() {
        let stream_options = object
            .entry("stream_options".to_string())
            .or_insert_with(|| json!({}));
        if let Some(opts) = stream_options.as_object_mut() {
            opts.insert("include_usage".to_string(), json!(true));
        } else {
            *stream_options = json!({ "include_usage": true });
        }
    }
    let usage_ctx = KiloUsageContext {
        provider_id: provider.id.clone(),
        outbound_model: outbound_model.clone(),
        session_id: session_id.clone(),
        is_stream,
    };
    let url = format!("{}/chat/completions", provider_config.base_url);
    let uri: Uri = url.parse().map_err(|error| {
        kilo_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_provider_config",
            format!("Invalid Kilo upstream URL: {error}"),
        )
    })?;
    let request_body = serde_json::to_vec(&body).map_err(|e| {
        kilo_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            e.to_string(),
        )
    })?;
    let mut outbound_headers = filtered_request_headers(&headers);
    outbound_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    outbound_headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", provider_config.api_key)).map_err(|e| {
            kilo_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_provider_config",
                e.to_string(),
            )
        })?,
    );
    for (name, value) in provider_config.headers.iter() {
        outbound_headers.insert(name.clone(), value.clone());
    }
    if let Some(host) = uri.host() {
        let host = uri
            .port()
            .map(|port| format!("{host}:{port}"))
            .unwrap_or_else(|| host.to_string());
        outbound_headers.insert(
            header::HOST,
            HeaderValue::from_str(&host).map_err(|e| {
                kilo_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invalid_provider_config",
                    e.to_string(),
                )
            })?,
        );
    }
    outbound_headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&request_body.len().to_string()).unwrap(),
    );

    let response = send_request(
        uri,
        "Kilo upstream /chat/completions",
        Method::POST,
        outbound_headers,
        extensions.clone(),
        request_body,
        timeout_for(&config),
        crate::proxy::http_client::get_current_proxy_url().as_deref(),
    )
    .await;
    let error = match response {
        Ok(response) if response.status().is_success() => {
            state.current_providers.write().await.insert(
                APP_TYPE.to_string(),
                (provider.id.clone(), provider.name.clone()),
            );
            {
                let mut status = state.status.write().await;
                status.current_provider = Some(provider.name.clone());
                status.current_provider_id = Some(provider.id.clone());
            }
            let response_ctx =
                ResponseContext::for_kilo(started, session_id.clone(), outbound_model.clone());
            match process_response(
                response,
                &response_ctx,
                &state,
                &KILO_PARSER_CONFIG,
                None,
                &provider.id,
            )
            .await
            {
                Ok(response) => {
                    record_success(&state, &provider, started, false).await;
                    return Ok(response);
                }
                Err(error) => error,
            }
        }
        Ok(response) => {
            let status = response.status();
            let body = response
                .bytes_with_limit(MAX_RESPONSE_BODY_BYTES)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&body).to_string();
            let _ = state
                .provider_router
                .record_result(
                    &provider.id,
                    APP_TYPE,
                    false,
                    false,
                    Some(format!("upstream status {status}")),
                )
                .await;
            ProxyError::UpstreamError {
                status: status.as_u16(),
                body: Some(body),
            }
        }
        Err(error) => {
            let _ = state
                .provider_router
                .record_result(
                    &provider.id,
                    APP_TYPE,
                    false,
                    false,
                    Some(error.to_string()),
                )
                .await;
            error
        }
    };
    let (status, code, message) = match &error {
        ProxyError::AuthError(_) => (
            StatusCode::UNAUTHORIZED,
            "upstream_auth_error",
            "Kilo upstream authentication failed".to_string(),
        ),
        ProxyError::ConfigError(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_provider_config",
            error.to_string(),
        ),
        ProxyError::UpstreamError { status, .. } if *status == 401 || *status == 403 => (
            StatusCode::UNAUTHORIZED,
            "upstream_auth_error",
            "Kilo upstream authentication failed".to_string(),
        ),
        ProxyError::UpstreamError { .. }
        | ProxyError::ForwardFailed(_)
        | ProxyError::Timeout(_) => (
            StatusCode::BAD_GATEWAY,
            "upstream_unavailable",
            "Kilo upstream is unavailable".to_string(),
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            "upstream_unavailable",
            "Kilo upstream is unavailable".to_string(),
        ),
    };
    {
        let mut metrics = state.status.write().await;
        metrics.failed_requests = metrics.failed_requests.saturating_add(1);
        metrics.last_error = Some(error.to_string());
        if metrics.total_requests > 0 {
            metrics.success_rate =
                (metrics.success_requests as f32 / metrics.total_requests as f32) * 100.0;
        }
    }
    // 旁路记录可归属到实际 Provider 的失败请求：保留请求数和上游 status code，
    // 不因缺少 usage 就丢失统计。统计写入失败不改变业务错误响应。
    let error_status_code = match &error {
        ProxyError::UpstreamError { status, .. } => *status,
        _ => 0,
    };
    spawn_kilo_error_log(
        &state,
        &usage_ctx.provider_id,
        &usage_ctx.outbound_model,
        usage_ctx.is_stream,
        Some(usage_ctx.session_id.clone()),
        error_status_code,
        error.to_string(),
        started.elapsed().as_millis() as u64,
    );
    Err(kilo_error(status, code, message))
}

fn decode_request_body(headers: &mut HeaderMap, body: Bytes) -> Result<Bytes, Response> {
    let Some(encoding) = get_content_encoding(headers) else {
        return Ok(body);
    };
    if !is_supported_content_encoding(&encoding) {
        return Err(kilo_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("Unsupported request content-encoding: {encoding}"),
        ));
    }
    let decompressed = decompress_body(&encoding, &body).map_err(|e| {
        kilo_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("Failed to decompress request body: {e}"),
        )
    })?;
    headers.remove(header::CONTENT_ENCODING);
    headers.remove(header::CONTENT_LENGTH);
    headers.remove(header::TRANSFER_ENCODING);
    Ok(Bytes::from(decompressed.unwrap_or_else(|| body.to_vec())))
}

fn filtered_request_headers(headers: &HeaderMap) -> HeaderMap {
    let mut filtered = HeaderMap::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "authorization"
                | "x-api-key"
                | "x-goog-api-key"
                | "host"
                | "content-length"
                | "content-encoding"
                | "transfer-encoding"
                | "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "proxy-connection"
                | "te"
                | "trailer"
                | "upgrade"
        ) {
            continue;
        }
        filtered.append(name.clone(), value.clone());
    }
    filtered
}

fn timeout_for(config: &crate::proxy::types::AppProxyConfig) -> Duration {
    Duration::from_secs(config.non_streaming_timeout.max(1) as u64)
}

async fn record_success(
    state: &ProxyState,
    provider: &Provider,
    started: std::time::Instant,
    used_half_open_permit: bool,
) {
    if let Err(error) = state
        .provider_router
        .record_result(&provider.id, APP_TYPE, used_half_open_permit, true, None)
        .await
    {
        log::warn!("[{TAG}] failed to record provider success: {error}");
    }
    let mut status = state.status.write().await;
    status.success_requests = status.success_requests.saturating_add(1);
    status.last_request_at = Some(chrono::Utc::now().to_rfc3339());
    status.last_error = None;
    if status.total_requests > 0 {
        status.success_rate =
            (status.success_requests as f32 / status.total_requests as f32) * 100.0;
    }
    let _ = started;
}

/// 异步记录 Kilo 失败请求的错误统计行。归属实际 provider_id，保留请求数和
/// 上游 status code。统计写入失败只记 warn，不改变业务错误响应。
fn spawn_kilo_error_log(
    state: &ProxyState,
    provider_id: &str,
    model: &str,
    is_streaming: bool,
    session_id: Option<String>,
    status_code: u16,
    error_message: String,
    latency_ms: u64,
) {
    if !usage_logging_enabled(state) {
        return;
    }
    let db = state.db.clone();
    let provider_id = provider_id.to_string();
    let model = model.to_string();
    let app_type = APP_TYPE.to_string();
    tokio::spawn(async move {
        let logger = UsageLogger::new(&db);
        if let Err(e) = logger.log_error_with_context(
            uuid::Uuid::new_v4().to_string(),
            provider_id,
            app_type,
            model,
            status_code,
            error_message,
            latency_ms,
            is_streaming,
            session_id,
            None,
        ) {
            log::warn!("[{TAG}] 记录 Kilo 错误统计失败: {e}");
        }
    });
}

fn kilo_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    let body = json!({
        "error": {
            "message": message.into(),
            "type": if status.is_server_error() { "proxy_error" } else { "invalid_request" },
            "code": code,
        }
    });
    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::{apply_provider_request_overrides, parse_provider, validate_provider_settings};
    use crate::provider::Provider;
    use serde_json::json;

    fn valid_settings() -> serde_json::Value {
        json!({
            "name": "Provider",
            "npm": "@ai-sdk/openai-compatible",
            "models": {
                "provider-model": {"name": "Provider Model"}
            },
            "thinking": {"type": "enabled"},
            "reasoning_effort": "high",
            "options": {
                "baseURL": "https://provider.example/v1",
                "apiKey": "secret",
                "headers": {"X-Tenant": "tenant-a"}
            }
        })
    }

    #[test]
    fn validates_standard_provider_shape() {
        validate_provider_settings(&valid_settings()).expect("valid Kilo provider");
    }

    #[test]
    fn parses_configured_reasoning_request_overrides() {
        let provider = Provider::with_id(
            "provider".to_string(),
            "Provider".to_string(),
            valid_settings(),
            None,
        );
        let config = parse_provider(&provider).expect("valid Kilo provider");

        assert_eq!(config.thinking, json!({ "type": "enabled" }));
        assert_eq!(config.reasoning_effort, json!("high"));
    }

    #[test]
    fn rejects_missing_reasoning_request_overrides() {
        let mut settings = valid_settings();
        settings.as_object_mut().unwrap().remove("thinking");
        assert!(validate_provider_settings(&settings).is_err());

        let mut settings = valid_settings();
        settings.as_object_mut().unwrap().remove("reasoning_effort");
        assert!(validate_provider_settings(&settings).is_err());
    }

    #[test]
    fn provider_reasoning_request_overrides_replace_client_values() {
        let mut settings = valid_settings();
        settings["thinking"] = json!({ "type": "enabled" });
        settings["reasoning_effort"] = json!("max");
        let provider = Provider::with_id(
            "provider".to_string(),
            "Provider".to_string(),
            settings,
            None,
        );
        let config = parse_provider(&provider).expect("valid Kilo provider");
        let mut body = json!({
            "model": "client-model",
            "thinking": { "type": "disabled" },
            "reasoning_effort": "low",
            "messages": []
        });

        apply_provider_request_overrides(&mut body, &config);

        assert_eq!(body["thinking"], json!({ "type": "enabled" }));
        assert_eq!(body["reasoning_effort"], json!("max"));
        assert_eq!(body["model"], "client-model");
        assert_eq!(body["messages"], json!([]));
    }

    #[test]
    fn rejects_proxy_recursion_and_auth_header_override() {
        let mut recursive = valid_settings();
        recursive["options"]["baseURL"] = json!("http://127.0.0.1:15721/kilo/v1");
        assert!(validate_provider_settings(&recursive).is_err());

        let mut auth_override = valid_settings();
        auth_override["options"]["headers"] = json!({"Authorization": "Bearer forged"});
        assert!(validate_provider_settings(&auth_override).is_err());
    }
}
