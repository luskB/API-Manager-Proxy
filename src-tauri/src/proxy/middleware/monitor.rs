use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use http_body_util::BodyExt;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::proxy::middleware::auth::AuthenticatedKey;
use crate::proxy::monitor::{ProxyMonitor, ProxyRequestLog};

/// Extension type set by handlers to pass the upstream URL to the monitor middleware.
#[derive(Clone)]
pub struct UpstreamUrl(pub String);

/// Extension type set by handlers to pass the effective upstream model.
#[derive(Clone)]
pub struct EffectiveModel(pub String);

/// Extension type set by handlers to pass the effective upstream request body snapshot.
#[derive(Clone)]
pub struct EffectiveRequestBody(pub String);

/// Extension type set by handlers to pass the selected upstream account id.
#[derive(Clone)]
pub struct EffectiveAccountId(pub String);

/// Max bytes of request/response body to store per log entry.
const BODY_TRUNCATE_BYTES: usize = 4096;

struct StreamLogContext {
    id: String,
    timestamp: i64,
    method: String,
    url: String,
    status: u16,
    started_at: Instant,
    model: Option<String>,
    account_id: Option<String>,
    upstream_url: Option<String>,
    client_ip: Option<String>,
    request_body: Option<String>,
    original_request_body: Option<String>,
    api_key: Option<String>,
}

/// Middleware that records request/response metadata into ProxyMonitor.
pub async fn monitor_middleware(
    State(monitor): State<Arc<ProxyMonitor>>,
    request: Request,
    next: Next,
) -> Response {
    if !monitor.is_enabled() {
        return next.run(request).await;
    }

    let method = request.method().to_string();
    let url = request.uri().path().to_string();
    let client_ip = extract_client_ip(&request);

    // Extract model and a truncated copy of the request body
    let (model, req_body_snapshot, request) = extract_model_from_request(request).await;

    // Grab authenticated key info (if present) before running downstream
    let auth_key = request.extensions().get::<AuthenticatedKey>().cloned();

    let start = Instant::now();
    let response = next.run(request).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    let status = response.status().as_u16();

    // Extract token usage and a truncated copy of the response body
    let (input_tokens, output_tokens, error_msg, resp_body_snapshot, response) =
        extract_usage_from_response(response).await;

    let effective_model = response
        .extensions()
        .get::<EffectiveModel>()
        .map(|m| m.0.clone())
        .or_else(|| model.clone());
    let effective_request_body = response
        .extensions()
        .get::<EffectiveRequestBody>()
        .map(|b| b.0.clone())
        .or_else(|| req_body_snapshot.clone());
    let effective_account_id = response
        .extensions()
        .get::<EffectiveAccountId>()
        .map(|account| account.0.clone());
    let upstream_url = response.extensions().get::<UpstreamUrl>().map(|u| u.0.clone());

    if is_sse_response(&response) {
        return wrap_sse_response(
            monitor.clone(),
            response,
            StreamLogContext {
                id: uuid::Uuid::new_v4().to_string(),
                timestamp: chrono::Utc::now().timestamp(),
                method,
                url,
                status,
                started_at: start,
                model: effective_model,
                account_id: effective_account_id,
                upstream_url,
                client_ip,
                request_body: effective_request_body,
                original_request_body: req_body_snapshot,
                api_key: auth_key.map(|k| k.key),
            },
        );
    }

    let log = ProxyRequestLog {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().timestamp(),
        method,
        url,
        status,
        duration_ms,
        model: effective_model.clone(),
        account_id: effective_account_id.clone(),
        upstream_url,
        client_ip,
        input_tokens,
        output_tokens,
        error: error_msg,
        estimated_cost: estimate_cost(
            effective_account_id.as_deref(),
            &effective_model,
            input_tokens,
            output_tokens,
        ),
        request_body: effective_request_body,
        original_request_body: req_body_snapshot,
        response_body: resp_body_snapshot,
        api_key: auth_key.map(|k| k.key),
    };

    // Record stats
    crate::proxy::proxy_stats::global().record(&log);

    monitor.add_log(log);

    response
}

fn is_sse_response(response: &Response) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|value| value.contains("text/event-stream"))
        .unwrap_or(false)
}

fn wrap_sse_response(
    monitor: Arc<ProxyMonitor>,
    response: Response,
    context: StreamLogContext,
) -> Response {
    let (parts, body) = response.into_parts();
    let mut upstream_stream = body.into_data_stream();
    let (tx, mut rx) = mpsc::channel::<Bytes>(32);

    tokio::spawn(async move {
        let mut preview = BytesMut::new();
        let mut pending = Vec::new();
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut error_msg = None;
        let mut client_disconnected = false;

        while let Some(chunk_result) = upstream_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    let remaining = BODY_TRUNCATE_BYTES.saturating_sub(preview.len());
                    if remaining > 0 {
                        preview.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                    }
                    parse_sse_usage_incremental(
                        &mut pending,
                        &chunk,
                        &mut input_tokens,
                        &mut output_tokens,
                        &mut error_msg,
                    );

                    if !client_disconnected && tx.send(chunk).await.is_err() {
                        client_disconnected = true;
                    }
                }
                Err(error) => {
                    if error_msg.is_none() {
                        error_msg = Some(error.to_string());
                    }
                    break;
                }
            }
        }

        if !pending.is_empty() {
            parse_sse_usage_line(
                &pending,
                &mut input_tokens,
                &mut output_tokens,
                &mut error_msg,
            );
        }

        let response_body = if preview.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&preview).into_owned())
        };

        let log = ProxyRequestLog {
            id: context.id,
            timestamp: context.timestamp,
            method: context.method,
            url: context.url,
            status: context.status,
            duration_ms: context.started_at.elapsed().as_millis() as u64,
            model: context.model.clone(),
            account_id: context.account_id.clone(),
            upstream_url: context.upstream_url,
            client_ip: context.client_ip,
            input_tokens,
            output_tokens,
            error: error_msg,
            estimated_cost: estimate_cost(
                context.account_id.as_deref(),
                &context.model,
                input_tokens,
                output_tokens,
            ),
            request_body: context.request_body,
            original_request_body: context.original_request_body,
            response_body,
            api_key: context.api_key,
        };

        crate::proxy::proxy_stats::global().record(&log);
        monitor.add_log(log);
    });

    let forwarded = async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, std::io::Error>(chunk);
        }
    };

    Response::from_parts(parts, Body::from_stream(forwarded))
}

fn extract_client_ip(request: &Request) -> Option<String> {
    // Try X-Forwarded-For first, then X-Real-Ip
    request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .or_else(|| {
            request
                .headers()
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
}

async fn extract_model_from_request(request: Request) -> (Option<String>, Option<String>, Request) {
    let (parts, body) = request.into_parts();

    // Only attempt to parse JSON bodies
    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("application/json") {
        return (None, None, Request::from_parts(parts, body));
    }

    // Collect body bytes (limit to 10MB for safety)
    match body.collect().await {
        Ok(collected) => {
            let bytes = collected.to_bytes();

            let model = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(String::from));

            let snapshot = truncate_body(&bytes);

            let new_body = Body::from(bytes);
            (model, Some(snapshot), Request::from_parts(parts, new_body))
        }
        Err(_) => (None, None, Request::from_parts(parts, Body::empty())),
    }
}

async fn extract_usage_from_response(
    response: Response,
) -> (Option<i32>, Option<i32>, Option<String>, Option<String>, Response) {
    let (parts, body) = response.into_parts();

    let content_type = parts
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("application/json") {
        return (
            None,
            None,
            None,
            None,
            Response::from_parts(parts, body),
        );
    }

    match body.collect().await {
        Ok(collected) => {
            let bytes = collected.to_bytes();

            let (input_tokens, output_tokens, error_msg) = parse_usage_and_error(&bytes);

            let snapshot = truncate_body(&bytes);

            let new_body = Body::from(bytes);
            (
                input_tokens,
                output_tokens,
                error_msg,
                Some(snapshot),
                Response::from_parts(parts, new_body),
            )
        }
        Err(_) => (None, None, None, None, Response::from_parts(parts, Body::empty())),
    }
}

fn parse_usage_and_error(bytes: &Bytes) -> (Option<i32>, Option<i32>, Option<String>) {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };

    parse_usage_and_error_value(&value)
}

#[cfg(test)]
#[cfg(test)]
fn parse_usage_from_sse(bytes: &Bytes) -> (Option<i32>, Option<i32>, Option<String>) {
    let payload = String::from_utf8_lossy(bytes);
    let mut input_tokens = None;
    let mut output_tokens = None;
    let mut error_msg = None;

    for line in payload.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }

        let data = trimmed.trim_start_matches("data:").trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }

        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let (next_input, next_output, next_error) = parse_usage_and_error_value(&value);
        input_tokens = next_input.or(input_tokens);
        output_tokens = next_output.or(output_tokens);
        error_msg = next_error.or(error_msg);
    }

    (input_tokens, output_tokens, error_msg)
}

fn parse_sse_usage_incremental(
    pending: &mut Vec<u8>,
    chunk: &Bytes,
    input_tokens: &mut Option<i32>,
    output_tokens: &mut Option<i32>,
    error_msg: &mut Option<String>,
) {
    pending.extend_from_slice(chunk);

    while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
        let line = pending.drain(..=index).collect::<Vec<_>>();
        parse_sse_usage_line(&line, input_tokens, output_tokens, error_msg);
    }
}

fn parse_sse_usage_line(
    line: &[u8],
    input_tokens: &mut Option<i32>,
    output_tokens: &mut Option<i32>,
    error_msg: &mut Option<String>,
) {
    let trimmed = String::from_utf8_lossy(line);
    let trimmed = trimmed.trim();
    if !trimmed.starts_with("data:") {
        return;
    }

    let data = trimmed.trim_start_matches("data:").trim();
    if data.is_empty() || data == "[DONE]" {
        return;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };

    let (next_input, next_output, next_error) = parse_usage_and_error_value(&value);
    *input_tokens = next_input.or(*input_tokens);
    *output_tokens = next_output.or(*output_tokens);
    *error_msg = next_error.or(error_msg.take());
}

fn parse_usage_and_error_value(
    value: &serde_json::Value,
) -> (Option<i32>, Option<i32>, Option<String>) {
    let prompt_tokens = first_i32(
        value,
        &[
            &["usage", "prompt_tokens"],
            &["usage", "input_tokens"],
            &["usage", "prompt_token_count"],
            &["usage_metadata", "prompt_token_count"],
            &["usageMetadata", "promptTokenCount"],
            &["usageMetadata", "promptTokens"],
        ],
    );

    let completion_tokens = first_i32(
        value,
        &[
            &["usage", "completion_tokens"],
            &["usage", "output_tokens"],
            &["usage", "completion_token_count"],
            &["usage_metadata", "candidates_token_count"],
            &["usageMetadata", "candidatesTokenCount"],
            &["usageMetadata", "completionTokenCount"],
            &["usageMetadata", "outputTokens"],
        ],
    );

    let total_tokens = first_i32(
        value,
        &[
            &["usage", "total_tokens"],
            &["usage", "total_token_count"],
            &["usage_metadata", "total_token_count"],
            &["usageMetadata", "totalTokenCount"],
            &["usageMetadata", "totalTokens"],
        ],
    );

    let input_tokens = prompt_tokens.or_else(|| {
        total_tokens
            .zip(completion_tokens)
            .map(|(total, output)| total.saturating_sub(output))
    });
    let output_tokens = completion_tokens.or_else(|| {
        total_tokens
            .zip(prompt_tokens)
            .map(|(total, input)| total.saturating_sub(input))
    });

    let error_msg = first_string(
        value,
        &[
            &["error", "message"],
            &["error", "details"],
            &["message"],
        ],
    );

    (input_tokens, output_tokens, error_msg)
}

fn first_i32(value: &serde_json::Value, paths: &[&[&str]]) -> Option<i32> {
    paths.iter().find_map(|path| value_at_path(value, path).and_then(value_as_i32))
}

fn first_string(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value_at_path(value, path).and_then(|item| item.as_str()).map(String::from))
}

fn value_at_path<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn value_as_i32(value: &serde_json::Value) -> Option<i32> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.as_str().and_then(|v| v.parse::<i64>().ok()))
        .map(|v| v as i32)
}

/// Estimate cost using the global price cache.
fn estimate_cost(
    account_id: Option<&str>,
    model: &Option<String>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
) -> Option<f64> {
    let model_name = model.as_deref()?;
    let input = input_tokens.unwrap_or(0);
    let output = output_tokens.unwrap_or(0);
    if let Some(account_id) = account_id {
        if let Some(cost) = crate::proxy::site_price_cache::global()
            .estimate_cost(account_id, model_name, input, output)
        {
            return Some(cost);
        }
    }
    if input == 0 && output == 0 {
        return None;
    }
    crate::proxy::price_cache::global().estimate_cost(model_name, input, output)
}

pub fn recompute_estimated_cost_from_log(log: &ProxyRequestLog) -> Option<f64> {
    estimate_cost(
        log.account_id.as_deref(),
        &log.model,
        log.input_tokens,
        log.output_tokens,
    )
}

/// Truncate a body to `BODY_TRUNCATE_BYTES` and return a UTF-8 string.
/// Non-UTF-8 bytes are replaced with the Unicode replacement character.
fn truncate_body(bytes: &Bytes) -> String {
    let slice = if bytes.len() > BODY_TRUNCATE_BYTES {
        &bytes[..BODY_TRUNCATE_BYTES]
    } else {
        bytes.as_ref()
    };
    String::from_utf8_lossy(slice).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_usage_supports_gemini_usage_metadata() {
        let bytes = Bytes::from_static(
            br#"{"usageMetadata":{"promptTokenCount":120,"candidatesTokenCount":30,"totalTokenCount":150}}"#,
        );

        let (input, output, error) = parse_usage_and_error(&bytes);
        assert_eq!(input, Some(120));
        assert_eq!(output, Some(30));
        assert_eq!(error, None);
    }

    #[test]
    fn parse_usage_supports_anthropic_stream_events() {
        let bytes = Bytes::from_static(
            b"event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":45,\"output_tokens\":18}}\n\ndata: [DONE]\n",
        );

        let (input, output, error) = parse_usage_from_sse(&bytes);
        assert_eq!(input, Some(45));
        assert_eq!(output, Some(18));
        assert_eq!(error, None);
    }

    #[test]
    fn parse_usage_supports_openai_stream_usage_chunk() {
        let bytes = Bytes::from_static(
            b"data: {\"usage\":{\"prompt_tokens\":64,\"completion_tokens\":16}}\n\ndata: [DONE]\n",
        );

        let (input, output, error) = parse_usage_from_sse(&bytes);
        assert_eq!(input, Some(64));
        assert_eq!(output, Some(16));
        assert_eq!(error, None);
    }

    #[tokio::test]
    async fn sse_logging_survives_client_disconnect() {
        let monitor = Arc::new(ProxyMonitor::new(10));
        let upstream = async_stream::stream! {
            yield Ok::<Bytes, std::io::Error>(Bytes::from_static(
                b"data: {\"usage\":{\"prompt_tokens\":12}}\n\n",
            ));
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            yield Ok::<Bytes, std::io::Error>(Bytes::from_static(
                b"data: {\"usage\":{\"completion_tokens\":5}}\n\ndata: [DONE]\n",
            ));
        };

        let response = Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(upstream))
            .unwrap();

        let wrapped = wrap_sse_response(
            monitor.clone(),
            response,
            StreamLogContext {
                id: "test-log".to_string(),
                timestamp: chrono::Utc::now().timestamp(),
                method: "POST".to_string(),
                url: "/v1/chat/completions".to_string(),
                status: 200,
                started_at: Instant::now(),
                model: Some("gpt-5.2".to_string()),
                account_id: None,
                upstream_url: None,
                client_ip: None,
                request_body: None,
                original_request_body: None,
                api_key: None,
            },
        );

        let (_parts, body) = wrapped.into_parts();
        let mut stream = body.into_data_stream();
        let _ = stream.next().await;
        drop(stream);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(monitor.get_count(), 1);
        let log = monitor.get_log("test-log").unwrap();
        assert_eq!(log.input_tokens, Some(12));
        assert_eq!(log.output_tokens, Some(5));
    }
}
