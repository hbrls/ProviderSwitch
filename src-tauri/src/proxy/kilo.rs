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
use std::time::{Duration, Instant};

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

use crate::error::AppError;
use crate::services::stream_check::{HealthStatus, StreamCheckConfig, StreamCheckResult};
use reqwest::Client;

/// Kilo 显式客户端工厂：由供应商配置精确决定直连或代理，不继承全局/系统/环境代理。
///
/// 供连通性检测等需要按 Provider `options.proxy` 精确控制代理的路径使用：
/// - `None` → 显式直连：`no_proxy()` 禁止环境变量 / 系统代理，本机目标也直连；
/// - `Some(url)` → 所有目标（含 localhost / 127.0.0.1 / ::1）一律走该代理，无绕过逻辑。
///
/// 与 全局客户端工厂 不同，这里不做业务层 scheme 白名单校验——地址能否构建、能否连接
/// 由网络库决定；也不设 client 总超时，单次请求超时由调用方 `.timeout()` 控制，
/// 后续流式转发可复用此工厂。构造失败必须上抛，禁止回退全局/直连 client。
pub(crate) fn build_http_client(proxy_url: Option<&str>) -> Result<Client, String> {
    let mut builder = Client::builder()
        // 先禁用自动代理：显式 no_proxy + 显式 Proxy::all 之外的来源一律不生效
        .no_proxy()
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .tcp_keepalive(Duration::from_secs(60))
        // 与全局客户端口径一致：禁用自动解压，禁用自动重定向
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::none());

    if let Some(url) = proxy_url {
        let proxy = reqwest::Proxy::all(url).map_err(|e| {
            format!(
                "Invalid proxy URL '{}': {}",
                crate::proxy::http_client::mask_url(url),
                e
            )
        })?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Provider 级 Kilo 代理：`settings_config.options.proxy`。
///
/// as_str → trim → 过滤空字符串；缺失、空串、纯空白、非字符串均为无值（直连）。
/// 非空值原样交给客户端工厂，不在此处做 scheme/host/port 等格式校验。
fn extract_proxy(provider: &Provider) -> Option<String> {
    provider
        .settings_config
        .get("options")
        .and_then(|value| value.get("proxy"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 按 Provider 的代理配置执行一次 GET 探测；任意 HTTP 响应均视为可达，不重试。
pub async fn check_provider(provider: &Provider) -> Result<StreamCheckResult, AppError> {
    let config = StreamCheckConfig::default();
    let started = Instant::now();
    let result: Result<u16, AppError> = async {
        let base_url = reachability_base_url(provider)?;
        let uri: Uri = base_url
            .parse()
            .map_err(|e| AppError::Message(format!("Invalid Kilo base URL: {e}")))?;
        let authority = uri
            .authority()
            .ok_or_else(|| AppError::Message("Kilo base URL has no host".to_string()))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_str(authority.as_str())
                .map_err(|e| AppError::Message(format!("Invalid Kilo host: {e}")))?,
        );
        headers.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        if let Some(ua) = provider
            .meta
            .as_ref()
            .and_then(|meta| meta.custom_user_agent_header().ok().flatten())
        {
            headers.insert(header::USER_AGENT, ua);
        }
        let mut extensions = http::Extensions::new();
        extensions.insert(crate::proxy::hyper_client::OriginalHeaderCases {
            cases: headers
                .keys()
                .map(|name| (name.as_str().to_string(), name.as_str().as_bytes().to_vec()))
                .collect(),
        });
        let proxy = extract_proxy(provider);
        let response = send_request(
            uri,
            "Kilo provider reachability check",
            Method::GET,
            headers,
            extensions,
            Vec::new(),
            Duration::from_secs(config.timeout_secs),
            proxy.as_deref(),
        )
        .await
        .map_err(|e| AppError::Message(e.to_string()))?;
        Ok(response.status().as_u16())
    }
    .await;
    let elapsed = started.elapsed().as_millis() as u64;
    let (status, success, message, http_status) = match result {
        Ok(code) => (
            if elapsed <= config.degraded_threshold_ms {
                HealthStatus::Operational
            } else {
                HealthStatus::Degraded
            },
            true,
            "Reachable".to_string(),
            Some(code),
        ),
        Err(error) => (HealthStatus::Failed, false, error.to_string(), None),
    };
    Ok(StreamCheckResult {
        status,
        success,
        message,
        response_time_ms: Some(elapsed),
        http_status,
        tested_at: chrono::Utc::now().timestamp(),
        retry_count: 0,
    })
}

pub(crate) fn reachability_base_url(provider: &Provider) -> Result<String, AppError> {
    provider
        .settings_config
        .get("options")
        .and_then(|value| value.as_object())
        .and_then(|options| options.get("baseURL"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "kilo_base_url_missing",
                "Kilo 供应商缺少 options.baseURL",
                "Kilo provider is missing `options.baseURL`",
            )
        })
}

/// 为 Kilo 连通性检测选择 Provider 自身的代理配置。
pub(crate) fn reachability_client(provider: &Provider) -> Result<Client, AppError> {
    build_http_client(extract_proxy(provider).as_deref())
        .map_err(|error| AppError::Message(format!("Failed to build Kilo HTTP client: {error}")))
}

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
    let mut extensions = parts.extensions;
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

    outbound_headers.insert(
        HeaderName::from_static("x-provider-switch"),
        HeaderValue::from_static("Kilo"),
    );
    // Keep the existing raw sender available even without captured header cases.
    let mut original_cases = extensions
        .get::<crate::proxy::hyper_client::OriginalHeaderCases>()
        .cloned()
        .unwrap_or_default();
    original_cases
        .cases
        .retain(|(name, _)| !name.eq_ignore_ascii_case("x-provider-switch"));
    original_cases.cases.push((
        "x-provider-switch".to_string(),
        b"X-Provider-Switch".to_vec(),
    ));
    extensions.insert(original_cases);
    let proxy = extract_proxy(&provider);

    let response = send_request(
        uri,
        "Kilo upstream /chat/completions",
        Method::POST,
        outbound_headers,
        extensions,
        request_body,
        timeout_for(&config),
        proxy.as_deref(),
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
                is_stream,
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
    use super::{
        apply_provider_request_overrides, build_http_client, parse_provider,
        validate_provider_settings,
    };
    use crate::provider::Provider;
    use serde_json::json;

    use std::{env, time::Duration};

    fn spawn_ok_server() -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });
        format!("http://{addr}/v1/models")
    }

    #[test]
    fn test_build_http_client_direct() {
        let result = build_http_client(None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_with_proxy() {
        let result = build_http_client(Some("http://127.0.0.1:7890"));
        assert!(result.is_ok());
        let result = build_http_client(Some("socks5://127.0.0.1:1080"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_http_client_rejects_invalid_proxy() {
        // 不做业务层 scheme 白名单：地址能否被构建由 reqwest 自身决定，
        // 这里只验证完全无法解析的输入会返回错误
        let result = build_http_client(Some("http://"));
        assert!(result.is_err(), "Should reject empty-host proxy URL");
        let result = build_http_client(Some("://missing-scheme"));
        assert!(result.is_err(), "Should reject unparseable proxy URL");
    }

    #[tokio::test]
    async fn test_kilo_client_proxy_never_bypasses_loopback() {
        let url = spawn_ok_server();
        // 本机目标配有代理时也必须走代理：代理不可连则整体失败，不回退直连
        let proxied = build_http_client(Some("http://127.0.0.1:1")).unwrap();
        assert!(proxied
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .is_err());
        // 显式直连（None）始终能到达本机服务
        let direct = build_http_client(None).unwrap();
        let resp = direct
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .expect("kilo direct client should reach the local server");
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_kilo_client_direct_ignores_env_proxy() {
        let keys = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ];
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, env::var(k).ok())).collect();
        for key in &keys {
            env::set_var(key, "http://127.0.0.1:1");
        }

        let url = spawn_ok_server();
        let direct = build_http_client(None).unwrap();
        let result = direct
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await;

        for (key, value) in saved {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }

        let resp = result.expect("kilo direct client must ignore env proxies");
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[test]
    fn test_extract_kilo_proxy_variants() {
        for (config, expected) in [
            (serde_json::json!({}), None),
            (serde_json::json!({ "options": {} }), None),
            (serde_json::json!({ "options": { "proxy": "" } }), None),
            (serde_json::json!({ "options": { "proxy": "   " } }), None),
            (serde_json::json!({ "options": { "proxy": 123 } }), None),
            (
                serde_json::json!({ "options": { "proxy": " http://127.0.0.1:7890 " } }),
                Some("http://127.0.0.1:7890".to_string()),
            ),
        ] {
            let p = Provider::with_id("test".into(), "Test".into(), config, None);
            assert_eq!(super::extract_proxy(&p), expected);
        }
    }

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
    fn validates_volcano_kilo_preset_shape() {
        // 火山 preset 模板 + 用户填写的 API Key 后生成的最终 JSON。
        // 模板本身（apiKey 为空）会被拒绝，密钥必须由用户提供。
        let mut template = json!({
            "models": {
                "glm-5.3": {"name": "GLM-5.3"}
            },
            "thinking": {"type": "enabled"},
            "reasoning_effort": "high",
            "options": {
                "baseURL": "https://ark.cn-beijing.volces.com/api/coding/v3",
                "apiKey": ""
            }
        });
        assert!(validate_provider_settings(&template).is_err());

        template["options"]["apiKey"] = json!("volc-secret");
        validate_provider_settings(&template).expect("valid Volcano Kilo preset");
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
