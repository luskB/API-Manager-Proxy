use chrono::{Datelike, Local, TimeZone};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use serde_json::{json, Value};

use crate::models::SiteAccount;

#[derive(Debug, Clone, Serialize)]
pub struct HubDetectionResult {
    pub account_id: String,
    pub site_name: String,
    pub site_url: String,
    pub site_type: String,
    pub status: String,
    pub error: Option<String>,
    pub balance: Option<f64>,
    pub today_usage: Option<f64>,
    pub today_prompt_tokens: Option<u64>,
    pub today_completion_tokens: Option<u64>,
    pub today_requests_count: Option<u64>,
    pub models: Vec<String>,
    pub has_checkin: bool,
    pub can_check_in: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HubCheckinResult {
    pub account_id: String,
    pub site_name: String,
    pub success: bool,
    pub message: String,
    pub reward: Option<f64>,
    pub site_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HubCheckinResponse {
    pub checkin: HubCheckinResult,
    pub detection: Option<HubDetectionResult>,
}

#[derive(Debug, Clone)]
pub struct BalanceSnapshot {
    pub balance: Option<f64>,
    pub today_usage: Option<f64>,
    pub today_prompt_tokens: Option<u64>,
    pub today_completion_tokens: Option<u64>,
    pub today_requests_count: Option<u64>,
}

fn base_url(site_url: &str) -> String {
    site_url.trim_end_matches('/').to_string()
}

fn build_user_id_headers(user_id: i64) -> HeaderMap {
    let id_str = user_id.to_string();
    let mut headers = HeaderMap::new();

    let names = [
        "New-API-User",
        "Veloera-User",
        "voapi-user",
        "User-id",
        "Rix-Api-User",
        "neo-api-user",
    ];

    for name in names {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&id_str),
        ) {
            headers.insert(header_name, header_value);
        }
    }

    headers
}

fn build_auth_headers(user_id: i64, access_token: &str) -> HeaderMap {
    let mut headers = build_user_id_headers(user_id);
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", access_token)) {
        headers.insert(AUTHORIZATION, value);
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn build_openai_headers(api_key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
        headers.insert(AUTHORIZATION, value);
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn value_to_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|v| v as f64))
        .or_else(|| value.as_u64().map(|v| v as f64))
        .or_else(|| value.as_str().and_then(|v| v.parse::<f64>().ok()))
}

fn value_to_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        .or_else(|| {
            value.as_str().and_then(|v| {
                if let Ok(parsed) = v.parse::<u64>() {
                    Some(parsed)
                } else {
                    v.parse::<f64>().ok().and_then(|raw| {
                        if raw.is_finite() && raw >= 0.0 {
                            Some(raw.round() as u64)
                        } else {
                            None
                        }
                    })
                }
            })
        })
}

fn first_f64(data: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| data.get(*key).and_then(value_to_f64))
}

fn first_u64(data: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| data.get(*key).and_then(value_to_u64))
}

fn parse_model_list(payload: &Value) -> Vec<String> {
    if let Some(arr) = payload.get("data").and_then(|v| v.as_array()) {
        let mut models: Vec<String> = arr
            .iter()
            .filter_map(|item| {
                if let Some(name) = item.as_str() {
                    Some(name.to_string())
                } else {
                    item.get("id")
                        .and_then(|id| id.as_str())
                        .map(|id| id.to_string())
                }
            })
            .collect();
        models.sort();
        models.dedup();
        return models;
    }

    if let Some(obj) = payload.get("data").and_then(|v| v.as_object()) {
        let mut models: Vec<String> = obj.keys().cloned().collect();
        models.sort();
        models.dedup();
        return models;
    }

    Vec::new()
}

fn parse_balance_snapshot(payload: &Value) -> BalanceSnapshot {
    let data = payload.get("data").cloned().unwrap_or_else(|| payload.clone());

    BalanceSnapshot {
        balance: first_f64(&data, &["quota", "balance", "remaining_quota", "remain_quota"]),
        today_usage: first_f64(
            &data,
            &[
                "today_quota_consumption",
                "today_consumption",
                "today_usage",
                "today_used_quota",
                "today_used_amount",
                "today_amount",
                "today_cost",
                "quota_used_today",
                "used_quota_today",
            ],
        ),
        today_prompt_tokens: first_u64(&data, &["today_prompt_tokens", "prompt_tokens"]),
        today_completion_tokens: first_u64(&data, &["today_completion_tokens", "completion_tokens"]),
        today_requests_count: first_u64(
            &data,
            &["today_requests_count", "requests_count", "request_count"],
        ),
    }
}

fn parse_today_usage_stat(payload: &Value) -> Option<f64> {
    let data = payload.get("data").cloned().unwrap_or_else(|| payload.clone());
    first_f64(
        &data,
        &[
            "quota",
            "today_quota_consumption",
            "today_consumption",
            "today_usage",
            "used_quota",
            "consumption",
        ],
    )
}

fn parse_today_usage_data(payload: &Value) -> Option<f64> {
    let entries = payload.get("data").and_then(|value| value.as_array())?;
    let total: f64 = entries
        .iter()
        .filter_map(|entry| entry.get("quota").and_then(value_to_f64))
        .sum();
    Some(total)
}

fn is_json_success(payload: &Value) -> bool {
    if let Some(success) = payload.get("success").and_then(|v| v.as_bool()) {
        return success;
    }
    true
}

fn extract_message(payload: &Value) -> Option<String> {
    payload
        .get("message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            payload
                .get("error")
                .and_then(|v| v.get("message"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
) -> Result<(u16, Value), String> {
    let response = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Read response failed: {}", e))?;

    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err(format!("Unexpected HTML response (HTTP {})", status));
    }

    let payload = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    Ok((status, payload))
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    body: Value,
) -> Result<(u16, Value), String> {
    let response = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Read response failed: {}", e))?;

    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err(format!("Unexpected HTML response (HTTP {})", status));
    }

    let payload = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    Ok((status, payload))
}

async fn request_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    headers: HeaderMap,
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let mut req = client.request(method, url).headers(headers);
    if let Some(payload) = body {
        req = req.json(&payload);
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Read response failed: {}", e))?;

    if text.trim_start().starts_with("<!DOCTYPE") || text.trim_start().starts_with("<html") {
        return Err(format!("Unexpected HTML response (HTTP {})", status));
    }

    let payload = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("JSON parse failed: {}", e))?;

    Ok((status, payload))
}

pub async fn fetch_models(client: &reqwest::Client, account: &SiteAccount) -> Result<Vec<String>, String> {
    let base = base_url(&account.site_url);
    let access_headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);

    if account.site_type == "sub2api" {
        return Ok(Vec::new());
    }

    if account.site_type == "one-hub" || account.site_type == "done-hub" {
        let (status, payload) = get_json(
            client,
            &format!("{}/api/available_model", base),
            access_headers,
        )
        .await?;

        if !(200..300).contains(&status) {
            return Err(format!("Fetch models failed (HTTP {})", status));
        }
        if !is_json_success(&payload) {
            return Err(extract_message(&payload).unwrap_or_else(|| "Fetch models failed".to_string()));
        }
        return Ok(parse_model_list(&payload));
    }

    let last_err: Option<String>;
    let user_models_url = format!("{}/api/user/models", base);
    match get_json(client, &user_models_url, access_headers.clone()).await {
        Ok((status, payload)) => {
            if (200..300).contains(&status) && is_json_success(&payload) {
                let models = parse_model_list(&payload);
                if !models.is_empty() {
                    return Ok(models);
                }
            }
            last_err = Some(
                extract_message(&payload)
                    .unwrap_or_else(|| format!("Fetch /api/user/models failed (HTTP {})", status)),
            );
        }
        Err(e) => {
            last_err = Some(e);
        }
    }

    let openai_token = account
        .account_info
        .api_key
        .as_deref()
        .unwrap_or(&account.account_info.access_token);
    let openai_headers = build_openai_headers(openai_token);
    let openai_url = format!("{}/v1/models", base);
    match get_json(client, &openai_url, openai_headers).await {
        Ok((status, payload)) => {
            if !(200..300).contains(&status) {
                return Err(format!("Fetch /v1/models failed (HTTP {})", status));
            }
            if !is_json_success(&payload) {
                return Err(extract_message(&payload).unwrap_or_else(|| "Fetch /v1/models failed".to_string()));
            }
            Ok(parse_model_list(&payload))
        }
        Err(e) => Err(last_err.unwrap_or(e)),
    }
}

pub async fn fetch_balance_snapshot(
    client: &reqwest::Client,
    account: &SiteAccount,
) -> Result<BalanceSnapshot, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let (status, payload) = get_json(client, &format!("{}/api/user/self", base), headers).await?;

    if !(200..300).contains(&status) {
        return Err(format!("Fetch account info failed (HTTP {})", status));
    }
    if !is_json_success(&payload) {
        return Err(extract_message(&payload).unwrap_or_else(|| "Fetch account info failed".to_string()));
    }

    Ok(parse_balance_snapshot(&payload))
}

pub async fn fetch_balance_overview(
    client: &reqwest::Client,
    account: &SiteAccount,
) -> Result<BalanceSnapshot, String> {
    let mut snapshot = fetch_balance_snapshot(client, account).await?;

    if let Ok(today_usage) = fetch_today_usage(client, account).await {
        if today_usage.is_some() {
            snapshot.today_usage = today_usage;
        }
    }

    Ok(snapshot)
}

async fn fetch_today_usage_stat(
    client: &reqwest::Client,
    account: &SiteAccount,
) -> Result<Option<f64>, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let now = Local::now();
    let start = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or(now);
    let url = format!(
        "{}/api/log/self/stat?type=2&start_timestamp={}&end_timestamp={}",
        base,
        start.timestamp(),
        now.timestamp()
    );

    let (status, payload) = get_json(client, &url, headers).await?;
    if !(200..300).contains(&status) {
        return Err(format!("Fetch daily usage failed (HTTP {})", status));
    }
    if !is_json_success(&payload) {
        return Err(extract_message(&payload).unwrap_or_else(|| "Fetch daily usage failed".to_string()));
    }

    Ok(parse_today_usage_stat(&payload))
}

async fn fetch_today_usage_data(
    client: &reqwest::Client,
    account: &SiteAccount,
) -> Result<Option<f64>, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let now = Local::now();
    let start = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or(now);
    let url = format!(
        "{}/api/data/self?start_timestamp={}&end_timestamp={}",
        base,
        start.timestamp(),
        now.timestamp()
    );

    let (status, payload) = get_json(client, &url, headers).await?;
    if !(200..300).contains(&status) {
        return Err(format!("Fetch daily usage data failed (HTTP {})", status));
    }
    if !is_json_success(&payload) {
        return Err(extract_message(&payload).unwrap_or_else(|| "Fetch daily usage data failed".to_string()));
    }

    Ok(parse_today_usage_data(&payload))
}

async fn fetch_today_usage(
    client: &reqwest::Client,
    account: &SiteAccount,
) -> Result<Option<f64>, String> {
    match fetch_today_usage_stat(client, account).await {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) | Err(_) => fetch_today_usage_data(client, account).await,
    }
}

pub async fn fetch_checkin_status(
    client: &reqwest::Client,
    account: &SiteAccount,
) -> Result<(bool, Option<bool>), String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);

    let status_url = format!("{}/api/status", base);
    let (status_code, status_payload) = get_json(client, &status_url, headers.clone()).await?;
    if !(200..300).contains(&status_code) || !is_json_success(&status_payload) {
        return Ok((false, None));
    }

    let status_data = status_payload.get("data").cloned().unwrap_or_else(|| status_payload.clone());
    let has_checkin = status_data
        .get("check_in_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || status_data
            .get("checkin_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

    if !has_checkin {
        return Ok((false, None));
    }

    let veloera_url = format!("{}/api/user/check_in_status", base);
    if let Ok((code, payload)) = get_json(client, &veloera_url, headers.clone()).await {
        if (200..300).contains(&code) && is_json_success(&payload) {
            let data = payload.get("data").cloned().unwrap_or_else(|| payload.clone());
            if let Some(can_check_in) = data.get("can_check_in").and_then(|v| v.as_bool()) {
                return Ok((true, Some(can_check_in)));
            }
        }
    }

    let now = chrono::Local::now();
    let month = format!("{:04}-{:02}", now.year(), now.month());
    let new_api_url = format!("{}/api/user/checkin?month={}", base, month);
    if let Ok((code, payload)) = get_json(client, &new_api_url, headers).await {
        if (200..300).contains(&code) && is_json_success(&payload) {
            let data = payload.get("data").cloned().unwrap_or_else(|| payload.clone());
            if let Some(checked_in_today) = data
                .get("stats")
                .and_then(|v| v.get("checked_in_today"))
                .and_then(|v| v.as_bool())
            {
                return Ok((true, Some(!checked_in_today)));
            }
        }
    }

    Ok((true, None))
}

pub async fn detect_account(
    client: &reqwest::Client,
    account: &SiteAccount,
    _include_details: bool,
) -> HubDetectionResult {
    let models_res = fetch_models(client, account).await;
    let balance_res = fetch_balance_snapshot(client, account).await;
    let today_usage_res = fetch_today_usage(client, account).await;
    let checkin_res = fetch_checkin_status(client, account).await;

    let mut errors: Vec<String> = Vec::new();
    let models = match models_res {
        Ok(list) => list,
        Err(e) => {
            errors.push(format!("models: {}", e));
            Vec::new()
        }
    };

    let snapshot = match balance_res {
        Ok(data) => Some(data),
        Err(e) => {
            errors.push(format!("balance: {}", e));
            None
        }
    };

    let today_usage = match today_usage_res {
        Ok(value) => value.or_else(|| snapshot.as_ref().and_then(|s| s.today_usage)),
        Err(e) => {
            tracing::debug!(
                account_id = %account.id,
                error = %e,
                "Daily usage stat endpoint unavailable; falling back to account snapshot"
            );
            snapshot.as_ref().and_then(|s| s.today_usage)
        }
    };

    let (has_checkin, can_check_in) = match checkin_res {
        Ok((has, can)) => (has, can),
        Err(e) => {
            errors.push(format!("checkin: {}", e));
            (false, None)
        }
    };

    HubDetectionResult {
        account_id: account.id.clone(),
        site_name: account.site_name.clone(),
        site_url: account.site_url.clone(),
        site_type: account.site_type.clone(),
        status: if errors.is_empty() {
            "success".to_string()
        } else {
            "failed".to_string()
        },
        error: if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        },
        balance: snapshot.as_ref().and_then(|s| s.balance),
        today_usage,
        today_prompt_tokens: snapshot.as_ref().and_then(|s| s.today_prompt_tokens),
        today_completion_tokens: snapshot.as_ref().and_then(|s| s.today_completion_tokens),
        today_requests_count: snapshot.as_ref().and_then(|s| s.today_requests_count),
        models,
        has_checkin,
        can_check_in,
    }
}

fn extract_token_list(payload: &Value) -> Vec<Value> {
    let mut items: Vec<Value> = if let Some(arr) = payload.as_array() {
        arr.clone()
    } else if let Some(arr) = payload
        .get("data")
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_array())
    {
        arr.clone()
    } else if let Some(arr) = payload
        .get("data")
        .and_then(|v| v.get("items"))
        .and_then(|v| v.as_array())
    {
        arr.clone()
    } else if let Some(arr) = payload.get("data").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = payload
        .get("data")
        .and_then(|v| v.get("list"))
        .and_then(|v| v.as_array())
    {
        arr.clone()
    } else if let Some(arr) = payload
        .get("data")
        .and_then(|v| v.get("tokens"))
        .and_then(|v| v.as_array())
    {
        arr.clone()
    } else {
        Vec::new()
    };

    for item in &mut items {
        if let Some(obj) = item.as_object_mut() {
            let group_is_empty = obj
                .get("group")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().is_empty())
                .unwrap_or(true);
            if group_is_empty {
                obj.insert("group".to_string(), Value::String("default".to_string()));
            }
        }
    }

    items
}

pub async fn list_api_tokens(client: &reqwest::Client, account: &SiteAccount) -> Result<Vec<Value>, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let endpoints = [
        format!("{}/api/token/?page=1&size=100&keyword=&order=-id", base),
        format!("{}/api/token/?p=1&size=100", base),
        format!("{}/api/token/?p=0&size=100", base),
        format!("{}/api/token/", base),
    ];

    let mut last_error: Option<String> = None;
    for endpoint in endpoints {
        match get_json(client, &endpoint, headers.clone()).await {
            Ok((status, payload)) => {
                if !(200..300).contains(&status) {
                    last_error = Some(format!("List tokens failed (HTTP {})", status));
                    continue;
                }
                if !is_json_success(&payload) {
                    last_error = extract_message(&payload);
                    continue;
                }
                return Ok(extract_token_list(&payload));
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "List tokens failed".to_string()))
}

pub async fn create_api_token(
    client: &reqwest::Client,
    account: &SiteAccount,
    token_data: &Value,
) -> Result<Vec<Value>, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let endpoints = [format!("{}/api/token/", base), format!("{}/api/token", base)];

    let mut last_error: Option<String> = None;
    for endpoint in endpoints {
        match post_json(client, &endpoint, headers.clone(), token_data.clone()).await {
            Ok((status, payload)) => {
                if !(200..300).contains(&status) {
                    last_error = Some(format!("Create token failed (HTTP {})", status));
                    continue;
                }
                if !is_json_success(&payload) {
                    last_error = extract_message(&payload);
                    continue;
                }
                return list_api_tokens(client, account).await;
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Create token failed".to_string()))
}

pub async fn delete_api_token(
    client: &reqwest::Client,
    account: &SiteAccount,
    token_identifier: &Value,
) -> Result<Vec<Value>, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);

    let id = token_identifier
        .get("id")
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i64().map(|i| i.to_string()))
        });
    let key = token_identifier
        .get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if id.is_none() && key.is_none() {
        return Err("Missing token id/key".to_string());
    }

    let mut candidates: Vec<(reqwest::Method, String, Option<Value>)> = Vec::new();
    if let Some(id_value) = id.as_ref() {
        candidates.push((
            reqwest::Method::DELETE,
            format!("{}/api/token/{}", base, id_value),
            None,
        ));
        candidates.push((
            reqwest::Method::DELETE,
            format!("{}/api/token/?id={}", base, id_value),
            None,
        ));
        candidates.push((
            reqwest::Method::POST,
            format!("{}/api/token/{}/delete", base, id_value),
            Some(json!({ "id": id_value })),
        ));
        candidates.push((
            reqwest::Method::POST,
            format!("{}/api/token/delete", base),
            Some(json!({ "id": id_value })),
        ));
    }
    if let Some(key_value) = key.as_ref() {
        candidates.push((
            reqwest::Method::DELETE,
            format!("{}/api/token/{}", base, key_value),
            None,
        ));
        candidates.push((
            reqwest::Method::DELETE,
            format!("{}/api/token/?key={}", base, key_value),
            None,
        ));
        candidates.push((
            reqwest::Method::POST,
            format!("{}/api/token/delete", base),
            Some(json!({ "key": key_value })),
        ));
    }

    let mut last_error: Option<String> = None;
    for (method, url, body) in candidates {
        match request_json(client, method, &url, headers.clone(), body).await {
            Ok((status, payload)) => {
                if !(200..300).contains(&status) {
                    last_error = Some(format!("Delete token failed (HTTP {})", status));
                    continue;
                }
                if !is_json_success(&payload) {
                    last_error = extract_message(&payload);
                    continue;
                }
                return list_api_tokens(client, account).await;
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Delete token failed".to_string()))
}

fn normalize_user_groups(payload: &Value) -> Value {
    if let Some(obj) = payload.get("data").and_then(|v| v.as_object()) {
        let first_value = obj.values().next();
        if let Some(first) = first_value {
            if first.get("name").is_some() || first.get("ratio").is_some() {
                let mut normalized = serde_json::Map::new();
                for (group_key, value) in obj {
                    let enabled = value
                        .get("enable")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if !enabled {
                        continue;
                    }
                    let desc = value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .or_else(|| value.get("desc").and_then(|v| v.as_str()))
                        .unwrap_or(group_key);
                    let ratio = value.get("ratio").and_then(|v| v.as_f64()).unwrap_or(1.0);
                    normalized.insert(group_key.clone(), json!({ "desc": desc, "ratio": ratio }));
                }
                return Value::Object(normalized);
            }
        }
        return Value::Object(obj.clone());
    }

    if let Some(arr) = payload.get("data").and_then(|v| v.as_array()) {
        let mut normalized = serde_json::Map::new();
        for item in arr {
            if let Some(name) = item.as_str() {
                normalized.insert(name.to_string(), json!({ "desc": name, "ratio": 1.0 }));
            }
        }
        return Value::Object(normalized);
    }

    if let Some(obj) = payload.as_object() {
        return Value::Object(obj.clone());
    }

    json!({})
}

pub async fn fetch_user_groups(client: &reqwest::Client, account: &SiteAccount) -> Result<Value, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let endpoints = [
        format!("{}/api/user/self/groups", base),
        format!("{}/api/user_group_map", base),
        format!("{}/api/group", base),
    ];

    let mut last_error: Option<String> = None;
    for endpoint in endpoints {
        match get_json(client, &endpoint, headers.clone()).await {
            Ok((status, payload)) => {
                if !(200..300).contains(&status) {
                    last_error = Some(format!("Fetch groups failed (HTTP {})", status));
                    continue;
                }
                if !is_json_success(&payload) {
                    last_error = extract_message(&payload);
                    continue;
                }
                return Ok(normalize_user_groups(&payload));
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Fetch groups failed".to_string()))
}

fn normalize_model_pricing(payload: &Value) -> Value {
    fn value_to_f64(value: &Value) -> Option<f64> {
        value
            .as_f64()
            .or_else(|| value.as_i64().map(|v| v as f64))
            .or_else(|| value.as_u64().map(|v| v as f64))
            .or_else(|| value.as_str().and_then(|v| v.parse::<f64>().ok()))
    }

    fn normalize_quota_type(value: Option<&Value>) -> i64 {
        match value {
            Some(v) if v.is_string() => {
                if v.as_str()
                    .map(|kind| kind.eq_ignore_ascii_case("times"))
                    .unwrap_or(false)
                {
                    1
                } else {
                    0
                }
            }
            Some(v) => v.as_i64().or_else(|| v.as_u64().map(|raw| raw as i64)).unwrap_or(0),
            None => 0,
        }
    }

    fn normalize_array_model_price(model: &Value, quota_type: i64) -> Value {
        let scalar_price = model.get("model_price");
        if quota_type == 1 {
            return scalar_price.cloned().unwrap_or_else(|| json!(0));
        }

        if let Some(object_price) = scalar_price.and_then(|value| value.as_object()) {
            let input = object_price
                .get("input")
                .cloned()
                .or_else(|| model.get("input").cloned())
                .unwrap_or_else(|| json!(0));
            let output = object_price
                .get("output")
                .cloned()
                .or_else(|| model.get("output").cloned())
                .or_else(|| {
                    let input_num = value_to_f64(&input)?;
                    let completion_ratio = model
                        .get("completion_ratio")
                        .and_then(value_to_f64)
                        .unwrap_or(1.0);
                    Some(json!(input_num * completion_ratio))
                })
                .unwrap_or_else(|| json!(0));
            return json!({ "input": input, "output": output });
        }

        let input = model
            .get("input")
            .cloned()
            .or_else(|| model.get("model_ratio").cloned())
            .or_else(|| {
                scalar_price.and_then(|value| {
                    if value_to_f64(value).unwrap_or(0.0) > 0.0 {
                        Some(value.clone())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| json!(0));
        let output = model
            .get("output")
            .cloned()
            .or_else(|| {
                let input_num = value_to_f64(&input)?;
                let completion_ratio = model
                    .get("completion_ratio")
                    .and_then(value_to_f64)
                    .unwrap_or(1.0);
                Some(json!(input_num * completion_ratio))
            })
            .or_else(|| {
                scalar_price.and_then(|value| {
                    if value_to_f64(value).unwrap_or(0.0) > 0.0 {
                        Some(value.clone())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| json!(0));

        json!({ "input": input, "output": output })
    }

    if let Some(arr) = payload.get("data").and_then(|v| v.as_array()) {
        let mut pricing = serde_json::Map::new();
        for model in arr {
            let model_name = model
                .get("model_name")
                .and_then(|v| v.as_str())
                .or_else(|| model.get("model").and_then(|v| v.as_str()));
            if let Some(name) = model_name {
                let quota_type = normalize_quota_type(model.get("quota_type"));
                pricing.insert(
                    name.to_string(),
                    json!({
                        "quota_type": quota_type,
                        "model_ratio": model.get("model_ratio").cloned().unwrap_or_else(|| json!(1)),
                        "model_price": normalize_array_model_price(model, quota_type),
                        "completion_ratio": model.get("completion_ratio").cloned().unwrap_or_else(|| json!(1)),
                        "enable_groups": model.get("enable_groups").cloned().unwrap_or_else(|| json!([])),
                        "model_description": model.get("model_description").cloned().unwrap_or_else(|| json!("")),
                    }),
                );
            }
        }
        return json!({ "data": pricing });
    }

    if let Some(obj) = payload.get("data").and_then(|v| v.as_object()) {
        let mut pricing = serde_json::Map::new();
        for (model_name, value) in obj {
            if value.get("price").is_some() {
                let input = value
                    .get("price")
                    .and_then(|v| v.get("input"))
                    .cloned()
                    .unwrap_or_else(|| json!(0));
                let output = value
                    .get("price")
                    .and_then(|v| v.get("output"))
                    .cloned()
                    .unwrap_or_else(|| json!(0));
                let price_type = value
                    .get("price")
                    .and_then(|v| v.get("type"))
                    .cloned()
                    .unwrap_or_else(|| json!("tokens"));
                pricing.insert(
                    model_name.clone(),
                    json!({
                        "quota_type": if price_type == "times" { 1 } else { 0 },
                        "type": price_type,
                        "model_ratio": 1,
                        "completion_ratio": 1,
                        "enable_groups": value.get("groups").cloned().unwrap_or_else(|| json!([])),
                        "model_price": {"input": input, "output": output},
                    }),
                );
            } else {
                pricing.insert(model_name.clone(), value.clone());
            }
        }
        return json!({ "data": pricing });
    }

    payload.clone()
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_model_pricing, parse_balance_snapshot, parse_today_usage_data,
        parse_today_usage_stat,
    };
    use serde_json::json;

    #[test]
    fn normalize_model_pricing_uses_model_ratio_for_token_pricing_rows() {
        let payload = json!({
            "data": [
                {
                    "model_name": "gpt-4.1",
                    "quota_type": 0,
                    "model_ratio": 0.8,
                    "model_price": 0,
                    "completion_ratio": 2.5,
                    "enable_groups": ["default"]
                }
            ]
        });

        let normalized = normalize_model_pricing(&payload);
        let row = &normalized["data"]["gpt-4.1"];

        assert_eq!(row["quota_type"], json!(0));
        assert_eq!(row["model_price"]["input"], json!(0.8));
        assert_eq!(row["model_price"]["output"], json!(2.0));
    }

    #[test]
    fn normalize_model_pricing_preserves_times_pricing_scalars() {
        let payload = json!({
            "data": [
                {
                    "model_name": "gemini-image",
                    "quota_type": "times",
                    "model_price": 15
                }
            ]
        });

        let normalized = normalize_model_pricing(&payload);
        let row = &normalized["data"]["gemini-image"];

        assert_eq!(row["quota_type"], json!(1));
        assert_eq!(row["model_price"], json!(15));
    }

    #[test]
    fn parse_balance_snapshot_accepts_string_fields_and_alias_keys() {
        let payload = json!({
            "data": {
                "quota": "500000",
                "today_used_quota": "125000",
                "today_requests_count": "9",
                "today_prompt_tokens": "12",
                "today_completion_tokens": "34"
            }
        });

        let snapshot = parse_balance_snapshot(&payload);

        assert_eq!(snapshot.balance, Some(500000.0));
        assert_eq!(snapshot.today_usage, Some(125000.0));
        assert_eq!(snapshot.today_requests_count, Some(9));
        assert_eq!(snapshot.today_prompt_tokens, Some(12));
        assert_eq!(snapshot.today_completion_tokens, Some(34));
    }

    #[test]
    fn parse_today_usage_stat_reads_quota_field() {
        let payload = json!({
            "success": true,
            "data": {
                "quota": 275000
            }
        });

        assert_eq!(parse_today_usage_stat(&payload), Some(275000.0));
    }

    #[test]
    fn parse_today_usage_data_sums_daily_quota_rows() {
        let payload = json!({
            "success": true,
            "data": [
                { "model_name": "gpt-4o", "quota": 120000 },
                { "model_name": "gpt-4.1", "quota": "30000" }
            ]
        });

        assert_eq!(parse_today_usage_data(&payload), Some(150000.0));
    }
}

pub async fn fetch_model_pricing(client: &reqwest::Client, account: &SiteAccount) -> Result<Value, String> {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);
    let endpoints = [
        format!("{}/api/pricing", base),
        format!("{}/api/available_model", base),
    ];

    let mut last_error: Option<String> = None;
    for endpoint in endpoints {
        match get_json(client, &endpoint, headers.clone()).await {
            Ok((status, payload)) => {
                if !(200..300).contains(&status) {
                    last_error = Some(format!("Fetch pricing failed (HTTP {})", status));
                    continue;
                }
                if !is_json_success(&payload) {
                    last_error = extract_message(&payload);
                    continue;
                }
                return Ok(normalize_model_pricing(&payload));
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Fetch pricing failed".to_string()))
}

pub async fn checkin_account(client: &reqwest::Client, account: &SiteAccount) -> HubCheckinResult {
    let base = base_url(&account.site_url);
    let headers = build_auth_headers(account.account_info.id, &account.account_info.access_token);

    let (supports_checkin, _) = match fetch_checkin_status(client, account).await {
        Ok(v) => v,
        Err(_) => (false, None),
    };

    if !supports_checkin {
        return HubCheckinResult {
            account_id: account.id.clone(),
            site_name: account.site_name.clone(),
            success: false,
            message: "Site does not support check-in".to_string(),
            reward: None,
            site_type: None,
        };
    }

    let endpoints = [
        (format!("{}/api/user/check_in", base), "veloera".to_string()),
        (format!("{}/api/user/checkin", base), "newapi".to_string()),
    ];

    let mut last_error: Option<String> = None;
    for (endpoint, endpoint_type) in endpoints {
        match post_json(client, &endpoint, headers.clone(), json!({})).await {
            Ok((status, payload)) => {
                if !(200..300).contains(&status) {
                    last_error = Some(format!("Check-in failed (HTTP {})", status));
                    continue;
                }

                let success = payload
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let message = extract_message(&payload).unwrap_or_else(|| {
                    if success {
                        "Check-in success".to_string()
                    } else {
                        "Check-in failed".to_string()
                    }
                });

                if success {
                    let reward = payload
                        .get("data")
                        .and_then(|v| v.get("reward"))
                        .and_then(|v| v.as_f64())
                        .or_else(|| {
                            payload
                                .get("data")
                                .and_then(|v| v.get("quota_awarded"))
                                .and_then(|v| v.as_f64())
                        });
                    return HubCheckinResult {
                        account_id: account.id.clone(),
                        site_name: account.site_name.clone(),
                        success: true,
                        message,
                        reward,
                        site_type: Some(endpoint_type),
                    };
                }

                last_error = Some(message);
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    HubCheckinResult {
        account_id: account.id.clone(),
        site_name: account.site_name.clone(),
        success: false,
        message: last_error.unwrap_or_else(|| "Check-in failed".to_string()),
        reward: None,
        site_type: None,
    }
}
