use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use bytes::Bytes;
use std::collections::HashSet;

use crate::proxy::handlers::common::{
    apply_retry_strategy, determine_retry_strategy, effective_max_retries, is_auth_error,
    rate_limit_duration_for_status, should_rotate_account,
};
use crate::proxy::middleware::auth::AuthenticatedKey;
use crate::proxy::middleware::monitor::{
    EffectiveAccountId, EffectiveModel, EffectiveRequestBody, UpstreamUrl,
};
use crate::proxy::server::AppState;

const MODEL_PREFIX_SEP: &str = "::";
const FORWARDED_BODY_SNAPSHOT_BYTES: usize = 4096;

fn model_with_account_prefix(account_selector: &str, model: &str) -> String {
    format!("{}{}{}", account_selector, MODEL_PREFIX_SEP, model)
}

fn split_account_prefixed_model(model: &str) -> Option<(String, String)> {
    let idx = model.find(MODEL_PREFIX_SEP)?;
    let (account, raw) = model.split_at(idx);
    let raw_model = &raw[MODEL_PREFIX_SEP.len()..];
    if account.is_empty() || raw_model.is_empty() {
        return None;
    }
    Some((account.to_string(), raw_model.to_string()))
}

fn strip_account_prefix_for_upstream(model: &str) -> String {
    split_account_prefixed_model(model)
        .map(|(_, raw_model)| raw_model)
        .unwrap_or_else(|| model.to_string())
}

/// POST /v1/chat/completions
pub async fn handle_chat_completions(
    State(state): State<AppState>,
    auth_key: Option<Extension<AuthenticatedKey>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_with_retry(
        state,
        auth_key.map(|Extension(key)| key),
        "/v1/chat/completions",
        headers,
        body,
    )
    .await
}

/// POST /v1/completions
pub async fn handle_completions(
    State(state): State<AppState>,
    auth_key: Option<Extension<AuthenticatedKey>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_with_retry(
        state,
        auth_key.map(|Extension(key)| key),
        "/v1/completions",
        headers,
        body,
    )
    .await
}

/// GET /v1/models — returns aggregated model list from all accounts.
pub async fn handle_list_models(
    State(state): State<AppState>,
    auth_key: Option<Extension<AuthenticatedKey>>,
) -> impl IntoResponse {
    let auth_key = auth_key.map(|Extension(key)| key);
    let allowed_accounts = auth_key_allowed_accounts(auth_key.as_ref());
    let missing_accounts = state.token_manager.active_accounts_missing_models();
    if !missing_accounts.is_empty() {
        tracing::info!(
            missing_accounts = missing_accounts.len(),
            "Fetching models for active accounts missing model metadata"
        );
        state
            .token_manager
            .fetch_models_for_accounts(&state.upstream, &missing_accounts)
            .await;
    }

    let tagged_models = state.token_manager.get_models_by_account().await;

    if !tagged_models.is_empty() {
        let mut data: Vec<serde_json::Value> = Vec::new();

        for row in tagged_models {
            if !key_allows_account(auth_key.as_ref(), &row.account_id) {
                continue;
            }
            for model in row.models {
                if !key_allows_model(auth_key.as_ref(), &model, &model) {
                    continue;
                }
                data.push(serde_json::json!({
                    "id": model_with_account_prefix(&row.account_selector, &model),
                    "object": "model",
                    "owned_by": row.site_name,
                }));
            }
        }

        return (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "object": "list",
                "data": data,
            })),
        )
            .into_response();
    }

    let models = state.token_manager.get_all_models().await;

    if models.is_empty() {
        // Fallback: try fetching from a single upstream (original behavior)
        let token = match state
            .token_manager
            .get_token_excluding_for_accounts(None, None, Some("openai"), &[], allowed_accounts.as_ref())
        {
            Some(t) => t,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    error_json("No available accounts"),
                )
                    .into_response()
            }
        };

        match state
            .upstream
            .forward(
                &token.site_url,
                "/v1/models",
                reqwest::Method::GET,
                HeaderMap::new(),
                Bytes::new(),
                token.upstream_credential(),
            )
            .await
        {
            Ok(resp) => return convert_reqwest_response(resp).await,
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    error_json(&format!("Upstream error: {}", e)),
                )
                    .into_response()
            }
        }
    }

    // Build OpenAI-compatible /v1/models response
    let data: Vec<serde_json::Value> = models
        .iter()
        .filter(|id| key_allows_model(auth_key.as_ref(), id, id))
        .map(|id| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "owned_by": "system",
            })
        })
        .collect();

    let response = serde_json::json!({
        "object": "list",
        "data": data,
    });

    (StatusCode::OK, axum::Json(response)).into_response()
}

pub async fn forward_with_retry(
    state: AppState,
    auth_key: Option<AuthenticatedKey>,
    path: &str,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (mut response, upstream_url, effective_model, effective_request_body, effective_account_id) =
        forward_with_retry_inner(state, auth_key, path, headers, body).await;
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

async fn forward_with_retry_inner(
    state: AppState,
    auth_key: Option<AuthenticatedKey>,
    path: &str,
    headers: HeaderMap,
    body: Bytes,
) -> (
    Response,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let is_stream = extract_stream_flag(&body);
    let original_model = extract_model(&body);
    let mut requested_raw_model: Option<String> = None;
    let mut forced_account_selector: Option<String> = None;
    let mut forced_account_id: Option<String> = None;
    let mut failed_accounts: Vec<String> = Vec::new();

    // Resolve model alias
    let (model, body) = if let Some(ref m) = original_model {
        let (alias_input, maybe_account) = if let Some((account_selector, raw_model)) = split_account_prefixed_model(m) {
            forced_account_selector = Some(account_selector.clone());
            forced_account_id = state.token_manager.resolve_account_selector(&account_selector);
            (raw_model, Some(account_selector))
        } else {
            (m.clone(), None)
        };
        requested_raw_model = Some(alias_input.clone());

        let resolved = state.model_router.resolve_alias(&alias_input);
        if resolved != *m {
            if let Some(account_selector) = maybe_account {
                tracing::info!(
                    original = %m,
                    forced_account = %account_selector,
                    forced_account_id = forced_account_id.as_deref().unwrap_or("(unresolved)"),
                    resolved_model = %resolved,
                    "Account-prefixed model resolved"
                );
            } else {
                tracing::info!(original = %m, resolved = %resolved, "Model alias resolved");
            }
            // Replace model in body
            let new_body = replace_model_in_body(&body, &resolved);
            (Some(resolved), new_body)
        } else {
            (original_model, body)
        }
    } else {
        (original_model, body)
    };

    let max_retries = effective_max_retries(
        state.token_manager.active_healthy_count(Some("openai")),
    );
    let allowed_accounts = auth_key_allowed_accounts(auth_key.as_ref());
    let mut last_site_url: Option<String> = None;
    let mut last_effective_model: Option<String> = None;
    let mut last_effective_request_body: Option<String> = None;
    let mut last_account_id: Option<String> = None;
    let body_template = body;
    let forced_route = forced_account_id.is_some();

    if let Some(raw_model) = requested_raw_model.as_deref() {
        let effective_model = model.as_deref().unwrap_or(raw_model);
        if !key_allows_model(auth_key.as_ref(), raw_model, effective_model) {
            return (
                (
                    StatusCode::FORBIDDEN,
                    error_json("This API key is not allowed to access the requested model"),
                )
                    .into_response(),
                last_site_url,
                last_effective_model,
                last_effective_request_body,
                last_account_id,
            );
        }
    }

    if forced_account_selector.is_some() && forced_account_id.is_none() {
        let message = format!(
            "Forced account {} is no longer active. Refresh the model list and try again.",
            forced_account_selector.as_deref().unwrap_or("(unknown)")
        );
            return (
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    error_json(&message),
                )
                    .into_response(),
                last_site_url,
                last_effective_model,
                last_effective_request_body,
                last_account_id,
            );
    }

    if let Some(account_id) = forced_account_id.as_deref() {
        if !key_allows_account(auth_key.as_ref(), account_id) {
            return (
                (
                    StatusCode::FORBIDDEN,
                    error_json("This API key is not allowed to access the selected site"),
                )
                    .into_response(),
                last_site_url,
                last_effective_model,
                last_effective_request_body,
                last_account_id,
            );
        }
    }

    for attempt in 0..=max_retries {
        let candidate = match forced_account_id.as_deref() {
            Some(account_id) => state.token_manager.get_token_for_account(
                account_id,
                model.as_deref(),
                Some("openai"),
                &failed_accounts,
            ),
            None => state.token_manager.get_token_excluding_for_accounts(
                None,
                model.as_deref(),
                Some("openai"),
                &failed_accounts,
                allowed_accounts.as_ref(),
            ),
        };

        let token = match candidate {
            Some(t) => t,
            None => {
                let message = forced_account_id
                    .as_deref()
                    .and_then(|account_id| {
                        state.token_manager.explain_account_unavailability(
                            account_id,
                            model.as_deref(),
                            Some("openai"),
                            &failed_accounts,
                        )
                    })
                    .or_else(|| {
                        forced_account_selector.as_deref().map(|selector| {
                            format!(
                                "Forced account {} is no longer active. Refresh the model list and try again.",
                                selector
                            )
                        })
                    })
                    .unwrap_or_else(|| "No available accounts".to_string());
                return (
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        error_json(&message),
                    )
                        .into_response(),
                    last_site_url,
                    last_effective_model,
                    last_effective_request_body,
                    last_account_id,
                );
            }
        };

        let request_body = if let Some(ref requested_model) = model {
            if let Some(concrete_model) = state
                .token_manager
                .resolve_model_for_account(&token.account_id, requested_model)
            {
                if concrete_model != *requested_model {
                    tracing::info!(
                        account_id = %token.account_id,
                        requested = %requested_model,
                        resolved = %concrete_model,
                        "Resolved model name for selected account"
                    );
                    replace_model_in_body(&body_template, &concrete_model)
                } else {
                    body_template.clone()
                }
            } else {
                body_template.clone()
            }
        } else {
            body_template.clone()
        };

        let request_body = if let Some(body_model) = extract_model(&request_body) {
            let outbound_model = strip_account_prefix_for_upstream(&body_model);
            if outbound_model != body_model {
                tracing::info!(
                    account_id = %token.account_id,
                    original_model = %body_model,
                    outbound_model = %outbound_model,
                    "Removed account prefix before forwarding upstream"
                );
                replace_model_in_body(&request_body, &outbound_model)
            } else {
                request_body
            }
        } else {
            request_body
        };

        last_effective_model = extract_model(&request_body);
        last_effective_request_body = Some(truncate_body_snapshot(&request_body));

        last_site_url = Some(token.site_url.clone());
        last_account_id = Some(token.account_id.clone());

        let result = state
            .upstream
            .forward(
                &token.site_url,
                path,
                reqwest::Method::POST,
                headers.clone(),
                request_body,
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
                            stream_response(resp),
                            last_site_url,
                            last_effective_model,
                            last_effective_request_body,
                            last_account_id,
                        );
                    }
                    return (
                        convert_reqwest_response(resp).await,
                        last_site_url,
                        last_effective_model,
                        last_effective_request_body,
                        last_account_id,
                    );
                }

                // 404 = model not found → mark account models as stale
                // and immediately remove the model from this account's registry
                // so future requests won't route to it for this model.
                if status == 404 {
                    crate::proxy::model_cache::global().mark_stale(&token.account_id);
                    if let Some(ref m) = model {
                        state.token_manager.remove_model_for_account(&token.account_id, m);
                    }
                }
                if should_rotate_account(status) {
                    // Auth errors: immediately disable the account, don't just rate-limit
                    if is_auth_error(status) {
                        state
                            .token_manager
                            .mark_auth_failed(&token.account_id, status);
                    } else {
                        let retry_after = resp
                            .headers()
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
                    if forced_route || attempt >= max_retries {
                        if forced_route {
                            tracing::warn!(
                                account_id = %token.account_id,
                                status,
                                "Forced account request failed; not rotating to another account"
                            );
                        }
                        return (
                            convert_reqwest_response(resp).await,
                            last_site_url,
                            last_effective_model,
                            last_effective_request_body,
                            last_account_id,
                        );
                    }

                    let strategy = determine_retry_strategy(status, "");
                    if !apply_retry_strategy(&strategy, attempt).await {
                        return (
                            convert_reqwest_response(resp).await,
                            last_site_url,
                            last_effective_model,
                            last_effective_request_body,
                            last_account_id,
                        );
                    }
                    tracing::warn!(
                        account_id = %token.account_id,
                        status,
                        attempt,
                        "Rotating account due to upstream error"
                    );
                    continue;
                }

                // Non-retryable error or max retries reached
                if is_auth_error(status) {
                    state
                        .token_manager
                        .mark_auth_failed(&token.account_id, status);
                    return (
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            error_json("All upstream accounts failed authentication"),
                        )
                            .into_response(),
                        last_site_url,
                        last_effective_model,
                        last_effective_request_body,
                        last_account_id,
                    );
                } else if status >= 500 {
                    state.token_manager.mark_failed(&token.account_id);
                }
                return (
                    convert_reqwest_response(resp).await,
                    last_site_url,
                    last_effective_model,
                    last_effective_request_body,
                    last_account_id,
                );
            }
            Err(e) => {
                // Connection-level failure (timeout, DNS, TCP refused, etc.)
                state
                    .token_manager
                    .mark_connection_failed(&token.account_id);
                failed_accounts.push(token.account_id.clone());
                tracing::error!(
                    account_id = %token.account_id,
                    error = %e,
                    attempt,
                    "Upstream request failed"
                );

                if forced_route || attempt >= max_retries {
                    if forced_route {
                        tracing::warn!(
                            account_id = %token.account_id,
                            error = %e,
                            "Forced account request failed; not rotating to another account"
                        );
                    }
                    return (
                        (
                            StatusCode::BAD_GATEWAY,
                            error_json(&format!("Upstream error: {}", e)),
                        )
                            .into_response(),
                        last_site_url,
                        last_effective_model,
                        last_effective_request_body,
                        last_account_id,
                    );
                }
            }
        }
    }

    (
        (
            StatusCode::BAD_GATEWAY,
            error_json("All retry attempts failed"),
        )
            .into_response(),
        last_site_url,
        last_effective_model,
        last_effective_request_body,
        last_account_id,
    )
}

fn extract_stream_flag(body: &Bytes) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

fn extract_model(body: &Bytes) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(String::from))
}

fn replace_model_in_body(body: &Bytes, new_model: &str) -> Bytes {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "model".to_string(),
                    serde_json::Value::String(new_model.to_string()),
                );
            }
            Bytes::from(serde_json::to_vec(&v).unwrap_or_else(|_| body.to_vec()))
        }
        Err(_) => body.clone(),
    }
}

fn truncate_body_snapshot(bytes: &Bytes) -> String {
    let slice = if bytes.len() > FORWARDED_BODY_SNAPSHOT_BYTES {
        &bytes[..FORWARDED_BODY_SNAPSHOT_BYTES]
    } else {
        bytes.as_ref()
    };
    String::from_utf8_lossy(slice).into_owned()
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

fn key_allows_account(auth_key: Option<&AuthenticatedKey>, account_id: &str) -> bool {
    auth_key.map(|key| key.allows_account(account_id)).unwrap_or(true)
}

fn key_allows_model(
    auth_key: Option<&AuthenticatedKey>,
    requested_model: &str,
    effective_model: &str,
) -> bool {
    auth_key
        .map(|key| key.allows_model(requested_model) || key.allows_model(effective_model))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_account_prefix_for_upstream_keeps_raw_model_only() {
        assert_eq!(
            strip_account_prefix_for_upstream("光速API::gpt-5.2-codex"),
            "gpt-5.2-codex"
        );
        assert_eq!(
            strip_account_prefix_for_upstream("gpt-5.2-codex"),
            "gpt-5.2-codex"
        );
    }

    #[test]
    fn replace_model_in_body_overwrites_prefixed_model() {
        let body = Bytes::from_static(
            br#"{"model":"\u5149\u901fAPI::gpt-5.2-codex","stream":true}"#,
        );
        let updated = replace_model_in_body(&body, "gpt-5.2-codex");
        let payload: serde_json::Value = serde_json::from_slice(&updated).unwrap();
        assert_eq!(payload.get("model").and_then(|v| v.as_str()), Some("gpt-5.2-codex"));
    }

    #[test]
    fn truncate_body_snapshot_preserves_json_preview() {
        let body = Bytes::from_static(br#"{"model":"gpt-5.2","stream":true}"#);
        assert_eq!(truncate_body_snapshot(&body), "{\"model\":\"gpt-5.2\",\"stream\":true}");
    }
}

fn stream_response(resp: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);

    let mut builder = Response::builder().status(status);

    // Copy headers from upstream response
    for (key, value) in resp.headers() {
        builder = builder.header(key.clone(), value.clone());
    }

    let stream = resp.bytes_stream();
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "Stream error")
                .into_response()
        })
}

async fn convert_reqwest_response(resp: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
    let headers = resp.headers().clone();

    match resp.bytes().await {
        Ok(body) => {
            let mut builder = Response::builder().status(status);
            for (key, value) in headers.iter() {
                builder = builder.header(key.clone(), value.clone());
            }
            builder
                .body(Body::from(body))
                .unwrap_or_else(|_| {
                    (StatusCode::INTERNAL_SERVER_ERROR, "Response error").into_response()
                })
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            error_json(&format!("Failed to read upstream body: {}", e)),
        )
            .into_response(),
    }
}

fn error_json(message: &str) -> String {
    serde_json::json!({
        "error": {
            "message": message,
            "type": "proxy_error",
            "code": "proxy_error"
        }
    })
    .to_string()
}
