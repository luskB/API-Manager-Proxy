use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use bytes::Bytes;
use futures::StreamExt;
use std::collections::HashSet;

use crate::proxy::handlers::common::{
    apply_retry_strategy, determine_retry_strategy, effective_max_retries, is_auth_error,
    merge_account_filters, rate_limit_duration_for_status, should_rotate_account,
};
use crate::proxy::handlers::protocol_convert::{
    anthropic_to_openai_request, openai_to_anthropic_response, StreamConverter,
};
use crate::proxy::middleware::auth::AuthenticatedKey;
use crate::proxy::middleware::monitor::{
    EffectiveAccountId, EffectiveModel, EffectiveRequestBody, UpstreamUrl,
};
use crate::proxy::server::AppState;

const FORWARDED_BODY_SNAPSHOT_BYTES: usize = 4096;

/// POST /v1/messages
///
/// Accepts an Anthropic-format request, converts it to OpenAI format,
/// forwards to the upstream New API site via `/v1/chat/completions`,
/// then converts the response back to Anthropic format.
pub async fn handle_messages(
    State(state): State<AppState>,
    auth_key: Option<Extension<AuthenticatedKey>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (mut response, upstream_url, effective_model, effective_request_body, effective_account_id) =
        handle_messages_inner(state, auth_key.map(|Extension(key)| key), body).await;
    if let Some(url) = upstream_url {
        response.extensions_mut().insert(UpstreamUrl(url));
    }
    if let Some(model) = effective_model {
        response.extensions_mut().insert(EffectiveModel(model));
    }
    if let Some(body) = effective_request_body {
        response.extensions_mut().insert(EffectiveRequestBody(body));
    }
    if let Some(account_id) = effective_account_id {
        response.extensions_mut().insert(EffectiveAccountId(account_id));
    }
    response
}

async fn handle_messages_inner(
    state: AppState,
    auth_key: Option<AuthenticatedKey>,
    body: Bytes,
) -> (
    Response,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    // Parse the Anthropic request body
    let anthropic_body: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                (
                    StatusCode::BAD_REQUEST,
                    error_json(&format!("Invalid JSON: {}", e)),
                )
                    .into_response(),
                None,
                None,
                None,
                None,
            );
        }
    };

    let is_stream = anthropic_body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let model = anthropic_body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    // Resolve model alias
    let model = if !model.is_empty() {
        let resolved = {
            let router = state.model_router.read().await;
            router.resolve_alias(&model)
        };
        if resolved != model {
            tracing::info!(original = %model, resolved = %resolved, "Anthropic: model alias resolved");
        }
        resolved
    } else {
        model
    };
    if !model.is_empty()
        && auth_key
            .as_ref()
            .map(|key| key.allows_model(&model))
            .unwrap_or(true)
            == false
    {
        return (
            anthropic_error_response(
                StatusCode::FORBIDDEN,
                "permission_error",
                "This API key is not allowed to access the requested model",
            ),
            None,
            Some(model.clone()),
            None,
            None,
        );
    }

    // Convert Anthropic request → OpenAI request
    let openai_body = anthropic_to_openai_request(&anthropic_body);
    let openai_bytes = Bytes::from(serde_json::to_vec(&openai_body).unwrap_or_default());
    let effective_model = if model.is_empty() {
        None
    } else {
        Some(model.clone())
    };
    let effective_request_body = Some(truncate_forwarded_body(&openai_bytes));

    // Use OpenAI protocol for upstream selection — all New API sites support /v1/chat/completions
    let mut failed_accounts: Vec<String> = Vec::new();
    let active_count = state.token_manager.active_healthy_count(Some("openai"));
    let max_retries = effective_max_retries(active_count);
    let allowed_accounts = auth_key_allowed_accounts(auth_key.as_ref());
    let preferred_route_accounts = if model.is_empty() {
        None
    } else {
        let router = state.model_router.read().await;
        router.preferred_accounts(&model)
    };
    let (candidate_accounts, has_route_override) =
        merge_account_filters(preferred_route_accounts.clone(), allowed_accounts.as_ref());
    let mut last_site_url: Option<String> = None;
    let mut last_account_id: Option<String> = None;

    if has_route_override
        && candidate_accounts
            .as_ref()
            .map(|account_ids| account_ids.is_empty())
            .unwrap_or(false)
    {
        return (
            anthropic_error_response(
                StatusCode::FORBIDDEN,
                "permission_error",
                "This API key is not allowed to access the selected site",
            ),
            None,
            effective_model.clone(),
            effective_request_body.clone(),
            None,
        );
    }

    tracing::info!(
        model = %model,
        is_stream,
        active_count,
        max_retries,
        "Anthropic→OpenAI: starting request"
    );

    for attempt in 0..=max_retries {
        let token = match state.token_manager.get_token_excluding_for_accounts(
            None,
            if model.is_empty() { None } else { Some(&model) },
            Some("openai"),
            &failed_accounts,
            candidate_accounts.as_ref(),
        ) {
            Some(t) => {
                tracing::debug!(
                    account_id = %t.account_id,
                    site_name = %t.site_name,
                    attempt,
                    "Anthropic→OpenAI: selected account"
                );
                t
            }
            None => {
                let message = preferred_route_accounts
                    .as_ref()
                    .and_then(|account_ids| account_ids.first())
                    .and_then(|account_id| {
                        state.token_manager.explain_account_unavailability(
                            account_id,
                            if model.is_empty() { None } else { Some(&model) },
                            Some("openai"),
                            &failed_accounts,
                        )
                    })
                    .unwrap_or_else(|| "No available accounts".to_string());
                tracing::warn!(
                    attempt,
                    failed_count = failed_accounts.len(),
                    model = %model,
                    reason = %message,
                    "Anthropic→OpenAI: no available accounts"
                );
                return (
                    anthropic_error_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "overloaded_error",
                        &message,
                    ),
                    last_site_url,
                    effective_model.clone(),
                    effective_request_body.clone(),
                    last_account_id.clone(),
                );
            }
        };

        last_site_url = Some(token.site_url.clone());
        last_account_id = Some(token.account_id.clone());

        let mut req_headers = HeaderMap::new();
        req_headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );

        let result = state
            .upstream
            .forward(
                &token.site_url,
                "/v1/chat/completions",
                reqwest::Method::POST,
                req_headers,
                openai_bytes.clone(),
                token.upstream_credential(),
            )
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status().as_u16();

                if status >= 200 && status < 300 {
                    state.token_manager.mark_success(&token.account_id);

                    if is_stream {
                        return (
                            convert_stream_response(resp, &model),
                            last_site_url,
                            effective_model.clone(),
                            effective_request_body.clone(),
                            Some(token.account_id.clone()),
                        );
                    }
                    return (
                        convert_non_stream_response(resp, &model).await,
                        last_site_url,
                        effective_model.clone(),
                        effective_request_body.clone(),
                        Some(token.account_id.clone()),
                    );
                }

                // Non-2xx: save headers before consuming body
                let resp_headers = resp.headers().clone();
                let error_body = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "(failed to read body)".to_string());
                tracing::warn!(
                    account_id = %token.account_id,
                    site_name = %token.site_name,
                    status,
                    attempt,
                    upstream_error = %error_body,
                    "Anthropic→OpenAI: upstream returned error"
                );

                // 404 = model not found → mark account models as stale
                // and immediately remove the model from this account's registry
                // so future requests won't route to it for this model.
                if status == 404 {
                    crate::proxy::model_cache::global().mark_stale(&token.account_id);
                    if !model.is_empty() {
                        state.token_manager.remove_model_for_account(&token.account_id, &model);
                    }
                }

                if should_rotate_account(status) && attempt < max_retries {
                    if is_auth_error(status) {
                        state
                            .token_manager
                            .mark_auth_failed(&token.account_id, status);
                    } else {
                        let retry_after = resp_headers
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .map(std::time::Duration::from_secs);

                        let cooldown = rate_limit_duration_for_status(status, retry_after);
                        state
                            .token_manager
                            .mark_rate_limited(&token.account_id, status, Some(cooldown));
                    }

                    failed_accounts.push(token.account_id.clone());
                    let strategy = determine_retry_strategy(status, "");
                    if !apply_retry_strategy(&strategy, attempt).await {
                        return (
                            anthropic_error_response(
                                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                                "api_error",
                                &format!("Upstream returned {}", status),
                            ),
                            last_site_url,
                            effective_model.clone(),
                            effective_request_body.clone(),
                            Some(token.account_id.clone()),
                        );
                    }
                    tracing::warn!(
                        account_id = %token.account_id,
                        status,
                        attempt,
                        "Rotating account for Anthropic→OpenAI request"
                    );
                    continue;
                }

                // Non-retryable or max retries reached
                if is_auth_error(status) {
                    state
                        .token_manager
                        .mark_auth_failed(&token.account_id, status);
                    return (
                        anthropic_error_response(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "overloaded_error",
                            "All upstream accounts failed authentication",
                        ),
                        last_site_url,
                        effective_model.clone(),
                        effective_request_body.clone(),
                        Some(token.account_id.clone()),
                    );
                } else if status >= 500 {
                    state.token_manager.mark_failed(&token.account_id);
                }

                return (
                    anthropic_error_response(
                        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                        "api_error",
                        &format!("Upstream returned {}", status),
                    ),
                    last_site_url,
                    effective_model.clone(),
                    effective_request_body.clone(),
                    Some(token.account_id.clone()),
                );
            }
            Err(e) => {
                state
                    .token_manager
                    .mark_connection_failed(&token.account_id);
                failed_accounts.push(token.account_id.clone());
                tracing::error!(
                    account_id = %token.account_id,
                    error = %e,
                    attempt,
                    "Anthropic→OpenAI upstream request failed"
                );

                if attempt >= max_retries {
                    return (
                        anthropic_error_response(
                            StatusCode::BAD_GATEWAY,
                            "api_error",
                            &format!("Upstream error: {}", e),
                        ),
                        last_site_url,
                        effective_model.clone(),
                        effective_request_body.clone(),
                        Some(token.account_id.clone()),
                    );
                }
            }
        }
    }

    (
        anthropic_error_response(
            StatusCode::BAD_GATEWAY,
            "overloaded_error",
            "All retry attempts failed",
        ),
        last_site_url,
        effective_model,
        effective_request_body,
        last_account_id,
    )
}

fn truncate_forwarded_body(bytes: &Bytes) -> String {
    let slice = if bytes.len() > FORWARDED_BODY_SNAPSHOT_BYTES {
        &bytes[..FORWARDED_BODY_SNAPSHOT_BYTES]
    } else {
        bytes.as_ref()
    };
    String::from_utf8_lossy(slice).into_owned()
}

/// Convert a successful non-streaming OpenAI response to Anthropic format.
async fn convert_non_stream_response(resp: reqwest::Response, model: &str) -> Response {
    let status = resp.status().as_u16();
    match resp.bytes().await {
        Ok(body) => {
            let openai_json: serde_json::Value =
                serde_json::from_slice(&body).unwrap_or_default();
            let anthropic_json = openai_to_anthropic_response(&openai_json, model);
            let anthropic_bytes = serde_json::to_vec(&anthropic_json).unwrap_or_default();

            Response::builder()
                .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
                .header("content-type", "application/json")
                .body(Body::from(anthropic_bytes))
                .unwrap_or_else(|_| {
                    (StatusCode::INTERNAL_SERVER_ERROR, "Response error").into_response()
                })
        }
        Err(e) => anthropic_error_response(
            StatusCode::BAD_GATEWAY,
            "api_error",
            &format!("Failed to read upstream body: {}", e),
        ),
    }
}

/// Convert a successful streaming OpenAI response to Anthropic SSE format.
fn convert_stream_response(resp: reqwest::Response, model: &str) -> Response {
    let model = model.to_string();
    let upstream_stream = resp.bytes_stream();

    let converted = async_stream::stream! {
        let mut converter = StreamConverter::new(model);
        let mut buffer = String::new();

        tokio::pin!(upstream_stream);

        while let Some(chunk_result) = upstream_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    let text = match std::str::from_utf8(&chunk) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };

                    buffer.push_str(text);

                    // Process complete SSE lines
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].trim_end_matches('\r').to_string();
                        buffer = buffer[pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        // Extract the data from "data: ..." lines
                        if let Some(data) = line.strip_prefix("data: ") {
                            let events = converter.process_chunk(data);
                            for event in events {
                                yield Ok::<Bytes, std::io::Error>(Bytes::from(format!("{}\n", event)));
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Error reading upstream stream");
                    break;
                }
            }
        }

        // Process any remaining data in buffer
        if !buffer.trim().is_empty() {
            for line in buffer.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data: ") {
                    let events = converter.process_chunk(data);
                    for event in events {
                        yield Ok::<Bytes, std::io::Error>(Bytes::from(format!("{}\n", event)));
                    }
                }
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(converted))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "Stream error").into_response()
        })
}

/// Build an Anthropic-format error response.
fn anthropic_error_response(status: StatusCode, error_type: &str, message: &str) -> Response {
    let body = serde_json::json!({
        "type": "error",
        "error": {
            "type": error_type,
            "message": message
        }
    });
    (status, axum::Json(body)).into_response()
}

fn error_json(message: &str) -> String {
    serde_json::json!({
        "type": "error",
        "error": {
            "type": "invalid_request_error",
            "message": message
        }
    })
    .to_string()
}

fn auth_key_allowed_accounts(auth_key: Option<&AuthenticatedKey>) -> Option<HashSet<String>> {
    auth_key.and_then(|key| {
        if key.allowed_account_ids.is_empty() {
            None
        } else {
            Some(key.allowed_account_ids.iter().cloned().collect())
        }
    })
}
