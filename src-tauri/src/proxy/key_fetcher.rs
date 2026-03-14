//! Fetch API Keys from upstream sites using access_token (management credential).
//!
//! The access_token is a management-plane credential used to query account info,
//! list tokens, etc. The API Key (e.g. `sk-xxx`) is the data-plane credential
//! used to call AI APIs (`/v1/chat/completions`, `/v1/messages`, etc.).
//!
//! This module bridges the two: given an access_token, it calls the upstream
//! token listing endpoint and extracts a usable API Key.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use crate::models::SiteAccount;

/// A single token entry from the upstream `/api/token/` response.
#[derive(Debug, Deserialize)]
struct TokenEntry {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    token_value: Option<String>,
    /// 1 = enabled, other values = disabled/expired.
    #[serde(default = "default_status")]
    status: i64,
}

fn default_status() -> i64 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyFetchScope {
    All,
    MissingOrMasked,
}

fn normalize_access_token(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.to_ascii_lowercase().starts_with("bearer ") {
        trimmed[7..].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn needs_api_key_refresh(account: &SiteAccount) -> bool {
    if account.site_type == "sub2api" {
        return false;
    }

    !has_usable_api_key(account.account_info.api_key.as_deref())
}

pub fn has_usable_api_key(value: Option<&str>) -> bool {
    value
        .map(|key| {
            let trimmed = key.trim();
            !trimmed.is_empty() && !trimmed.contains('*')
        })
        .unwrap_or(false)
}

fn is_masked_api_key(value: &str) -> bool {
    value.trim().contains('*')
}

/// Build the fan-out user-id headers that various New-API forks expect.
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

/// Extract token entries from the upstream JSON response.
///
/// Handles multiple response shapes:
/// - Direct array: `[{ "key": "sk-xxx", "status": 1 }, ...]`
/// - New-API paginated: `{ "data": { "items": [...] } }`
/// - New-API fork paginated: `{ "data": { "data": [...] } }`
/// - OneHub paginated: `{ "data": [...] }`
fn parse_token_entries(body: &serde_json::Value) -> Vec<TokenEntry> {
    // Try: top-level is an array
    if let Some(arr) = body.as_array() {
        return arr
            .iter()
            .filter_map(|v| serde_json::from_value::<TokenEntry>(v.clone()).ok())
            .collect();
    }

    // Try: { "data": ... }
    if let Some(data) = body.get("data") {
        // data is array directly
        if let Some(arr) = data.as_array() {
            return arr
                .iter()
                .filter_map(|v| serde_json::from_value::<TokenEntry>(v.clone()).ok())
                .collect();
        }
        // data is object — try "items" then "data" as the array key
        for key in &["items", "data"] {
            if let Some(arr) = data.get(*key).and_then(|i| i.as_array()) {
                return arr
                    .iter()
                    .filter_map(|v| serde_json::from_value::<TokenEntry>(v.clone()).ok())
                    .collect();
            }
        }
    }

    Vec::new()
}

impl TokenEntry {
    fn candidate_key(&self) -> Option<&str> {
        [
            self.key.as_deref(),
            self.token.as_deref(),
            self.value.as_deref(),
            self.secret.as_deref(),
            self.token_value.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
    }
}

/// Select the first usable API key from a list of token entries.
/// A token is usable if `status == 1` and the upstream returned a non-masked key.
fn select_first_usable_key(entries: &[TokenEntry]) -> Option<String> {
    entries
        .iter()
        .filter(|t| t.status == 1)
        .filter_map(|t| t.candidate_key())
        .find(|key| has_usable_api_key(Some(*key)))
        .map(str::to_string)
}

fn has_only_masked_enabled_keys(entries: &[TokenEntry]) -> bool {
    let mut saw_enabled_candidate = false;

    for entry in entries.iter().filter(|t| t.status == 1) {
        let Some(candidate) = entry.candidate_key() else {
            continue;
        };

        saw_enabled_candidate = true;
        if !is_masked_api_key(candidate) {
            return false;
        }
    }

    saw_enabled_candidate
}

/// Fetch a usable API Key for the given account from its upstream site.
///
/// - For `sub2api`: returns the access_token as-is (the JWT is the API key).
/// - For all other site types: calls `GET /api/token/` with the access_token
///   and user-id headers, then picks the first enabled key.
pub async fn fetch_api_key(
    client: &reqwest::Client,
    site_url: &str,
    site_type: &str,
    access_token: &str,
    user_id: i64,
) -> Result<String, String> {
    // Sub2API: JWT doubles as the API key — no separate fetch needed.
    if site_type == "sub2api" {
        return Ok(access_token.to_string());
    }

    let url = format!(
        "{}/api/token/?p=0&size=100",
        site_url.trim_end_matches('/')
    );

    let mut headers = build_user_id_headers(user_id);
    if let Ok(auth_value) = HeaderValue::from_str(access_token) {
        headers.insert(reqwest::header::AUTHORIZATION, auth_value);
    }
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    let resp = client
        .get(&url)
        .headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch tokens from {}: {}", site_url, e))?;

    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(format!(
            "Auth failed fetching tokens from {} (HTTP {})",
            site_url, status
        ));
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "Unexpected status {} fetching tokens from {}",
            status, site_url
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response from {}: {}", site_url, e))?;

    // Some upstreams return HTTP 200 with {"success": false, "message": "..."}
    // when the access_token is invalid/expired. Surface their message directly.
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("{} responded: {}", site_url, msg));
    }

    let entries = parse_token_entries(&body);
    if entries.is_empty() {
        tracing::debug!(site_url, body = %body, "Token response parsed to empty list");
        return Err(format!("No tokens returned from {}", site_url));
    }

    select_first_usable_key(&entries)
        .ok_or_else(|| {
            if has_only_masked_enabled_keys(&entries) {
                format!(
                    "{} only exposes masked API keys via the token API",
                    site_url
                )
            } else {
                format!("No enabled tokens found on {}", site_url)
            }
        })
}

pub async fn populate_api_keys_for_accounts(
    accounts: &mut [SiteAccount],
    timeout: Duration,
    scope: ApiKeyFetchScope,
) {
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to create HTTP client for key fetching: {}", e);
            return;
        }
    };

    let mut handles = Vec::new();

    for (idx, account) in accounts.iter().enumerate() {
        if matches!(scope, ApiKeyFetchScope::MissingOrMasked) && !needs_api_key_refresh(account) {
            continue;
        }

        let client = client.clone();
        let site_url = account.site_url.clone();
        let site_type = account.site_type.clone();
        let access_token = normalize_access_token(&account.account_info.access_token);
        let user_id = account.account_info.id;
        let account_id = account.id.clone();

        handles.push(tokio::spawn(async move {
            let result = fetch_api_key(
                &client,
                &site_url,
                &site_type,
                &access_token,
                user_id,
            )
            .await;
            (idx, account_id, result)
        }));
    }

    for handle in handles {
        if let Ok((idx, account_id, result)) = handle.await {
            let had_usable_key_before = has_usable_api_key(accounts[idx].account_info.api_key.as_deref());
            match result {
                Ok(api_key) => {
                    tracing::info!(account_id = %account_id, "Fetched API key");
                    accounts[idx].account_info.api_key = Some(api_key);
                }
                Err(e) => {
                    tracing::warn!(account_id = %account_id, error = %e, "Failed to fetch API key");
                    if !had_usable_key_before {
                        accounts[idx].account_info.api_key = None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AccountInfo, SiteAccount};

    #[test]
    fn parse_direct_array() {
        let json = serde_json::json!([
            { "key": "sk-aaa", "status": 1 },
            { "key": "sk-bbb", "status": 2 },
            { "key": "sk-ccc", "status": 1 },
        ]);
        let entries = parse_token_entries(&json);
        assert_eq!(entries.len(), 3);
        assert_eq!(select_first_usable_key(&entries), Some("sk-aaa".to_string()));
    }

    #[test]
    fn parse_newapi_paginated() {
        let json = serde_json::json!({
            "success": true,
            "data": {
                "items": [
                    { "key": "sk-disabled", "status": 2 },
                    { "key": "sk-good", "status": 1 },
                ]
            }
        });
        let entries = parse_token_entries(&json);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            select_first_usable_key(&entries),
            Some("sk-good".to_string())
        );
    }

    #[test]
    fn parse_onehub_data_array() {
        let json = serde_json::json!({
            "data": [
                { "token": "sk-hub1", "status": 1 },
                { "key": "sk-hub2", "status": 1 },
            ]
        });
        let entries = parse_token_entries(&json);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            select_first_usable_key(&entries),
            Some("sk-hub1".to_string())
        );
    }

    #[test]
    fn parse_empty_response() {
        let json = serde_json::json!({ "data": { "items": [] } });
        let entries = parse_token_entries(&json);
        assert!(entries.is_empty());
        assert_eq!(select_first_usable_key(&entries), None);
    }

    #[test]
    fn parse_all_disabled() {
        let json = serde_json::json!([
            { "key": "sk-off1", "status": 0 },
            { "key": "sk-off2", "status": 3 },
        ]);
        let entries = parse_token_entries(&json);
        assert_eq!(entries.len(), 2);
        assert_eq!(select_first_usable_key(&entries), None);
    }

    #[test]
    fn parse_empty_key_skipped() {
        let json = serde_json::json!([
            { "key": "", "status": 1 },
            { "value": "sk-real", "status": 1 },
        ]);
        let entries = parse_token_entries(&json);
        assert_eq!(
            select_first_usable_key(&entries),
            Some("sk-real".to_string())
        );
    }

    #[test]
    fn masked_keys_are_not_treated_as_usable() {
        let json = serde_json::json!([
            { "key": "sk-abcd****wxyz", "status": 1 },
            { "token": "  ", "status": 1 }
        ]);
        let entries = parse_token_entries(&json);
        assert_eq!(select_first_usable_key(&entries), None);
        assert!(has_only_masked_enabled_keys(&entries));
    }

    #[test]
    fn parse_newapi_fork_data_data() {
        // Some New-API forks use { "data": { "data": [...] } } instead of "items"
        let json = serde_json::json!({
            "success": true,
            "message": "",
            "data": {
                "data": [
                    { "id": 600, "key": "sk-fork1", "status": 1, "name": "KEY" },
                    { "id": 601, "key": "sk-fork2", "status": 2, "name": "KEY2" },
                ],
                "page": 1,
                "size": 100,
                "total_count": 2
            }
        });
        let entries = parse_token_entries(&json);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            select_first_usable_key(&entries),
            Some("sk-fork1".to_string())
        );
    }

    #[test]
    fn build_user_id_headers_contains_all() {
        let headers = build_user_id_headers(42);
        assert_eq!(headers.get("New-API-User").unwrap(), "42");
        assert_eq!(headers.get("Veloera-User").unwrap(), "42");
        assert_eq!(headers.get("voapi-user").unwrap(), "42");
        assert_eq!(headers.get("User-id").unwrap(), "42");
        assert_eq!(headers.get("Rix-Api-User").unwrap(), "42");
        assert_eq!(headers.get("neo-api-user").unwrap(), "42");
    }

    #[test]
    fn normalize_access_token_strips_bearer_prefix() {
        assert_eq!(normalize_access_token("Bearer abc123"), "abc123");
        assert_eq!(normalize_access_token(" bearer xyz "), "xyz");
        assert_eq!(normalize_access_token("plain-token"), "plain-token");
    }

    #[test]
    fn needs_api_key_refresh_for_missing_or_masked_key() {
        let account = SiteAccount {
            id: "acc-1".to_string(),
            site_name: "Test".to_string(),
            site_url: "https://example.com".to_string(),
            site_type: "new-api".to_string(),
            account_info: AccountInfo {
                id: 1,
                access_token: "token".to_string(),
                api_key: None,
                username: String::new(),
                quota: 0.0,
                today_prompt_tokens: 0,
                today_completion_tokens: 0,
                today_quota_consumption: 0.0,
                today_requests_count: 0,
                today_income: 0.0,
            },
            auth_type: "access_token".to_string(),
            last_sync_time: 0,
            updated_at: 0,
            created_at: 0,
            notes: None,
            disabled: Some(false),
            health: None,
            exchange_rate: None,
            browser_profile_mode: None,
            browser_profile_path: None,
            proxy_health: None,
            proxy_priority: 0,
            proxy_weight: 10,
        };

        assert!(needs_api_key_refresh(&account));

        let mut masked = account.clone();
        masked.account_info.api_key = Some("sk-abc****xyz".to_string());
        assert!(needs_api_key_refresh(&masked));

        let mut usable = account.clone();
        usable.account_info.api_key = Some("sk-real-key".to_string());
        assert!(!needs_api_key_refresh(&usable));

        let mut sub2api = account;
        sub2api.site_type = "sub2api".to_string();
        assert!(!needs_api_key_refresh(&sub2api));
    }

    #[test]
    fn usable_api_key_requires_unmasked_value() {
        assert!(has_usable_api_key(Some("sk-real-key")));
        assert!(!has_usable_api_key(Some("sk-abc****xyz")));
        assert!(!has_usable_api_key(Some("   ")));
        assert!(!has_usable_api_key(None));
    }
}
