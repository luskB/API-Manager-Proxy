use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::models::{AppConfig, SiteAccount};
use crate::modules::{backup, browser_login, browser_storage, config, desktop, hub_service};
use crate::proxy::key_fetcher::{has_usable_api_key, populate_api_keys_for_accounts, ApiKeyFetchScope};
use crate::proxy::middleware::SecurityConfig;
use crate::proxy::monitor::ProxyMonitor;
use crate::proxy::proxy_stats::StatsScope;
use crate::proxy::server::ProxyServerHandle;
use crate::proxy::token_manager::{AccountModels, TokenManager};

/// Managed state for proxy server lifecycle.
#[derive(Clone)]
pub struct ProxyServiceState {
    pub server: Arc<Mutex<ProxyServerHandle>>,
    pub monitor: Arc<tokio::sync::RwLock<Option<Arc<ProxyMonitor>>>>,
    pub token_manager: Arc<tokio::sync::RwLock<Option<Arc<TokenManager>>>>,
    pub security: Arc<tokio::sync::RwLock<Option<Arc<tokio::sync::RwLock<SecurityConfig>>>>>,
}

impl ProxyServiceState {
    pub fn new() -> Self {
        Self {
            server: Arc::new(Mutex::new(ProxyServerHandle::new())),
            monitor: Arc::new(tokio::sync::RwLock::new(None)),
            token_manager: Arc::new(tokio::sync::RwLock::new(None)),
            security: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

// ============================================================================
// Tauri Commands
// ============================================================================

#[tauri::command]
pub async fn import_backup(path: String) -> Result<Vec<crate::models::SiteAccount>, String> {
    let mut accounts = backup::import_backup_from_path(std::path::Path::new(&path))?;
    fetch_api_keys_for_accounts(&mut accounts).await;
    Ok(accounts)
}

#[tauri::command]
pub async fn import_backup_from_text(json: String) -> Result<Vec<crate::models::SiteAccount>, String> {
    let mut accounts = backup::import_backup_from_str(&json)?;
    fetch_api_keys_for_accounts(&mut accounts).await;
    Ok(accounts)
}

#[tauri::command]
pub async fn detect_browser_extension() -> Result<serde_json::Value, String> {
    let dirs = browser_storage::discover_extension_dirs();

    let profiles: Vec<serde_json::Value> = dirs
        .iter()
        .map(|info| {
            serde_json::json!({
                "profile_name": info.profile_name,
                "extension_id": info.extension_id,
                "path": info.path.display().to_string(),
            })
        })
        .collect();

    let found = !dirs.is_empty();
    let extension_id = dirs.first().map(|d| d.extension_id.as_str()).unwrap_or("");

    Ok(serde_json::json!({
        "found": found,
        "profiles": profiles,
        "extension_id": extension_id,
    }))
}

#[tauri::command]
pub async fn sync_from_browser() -> Result<Vec<SiteAccount>, String> {
    // 1. Read raw JSON from Chrome LevelDB
    let raw_json = browser_storage::read_accounts_from_browser()?;

    // 2. Parse the AccountStorageConfig to extract the accounts array.
    //    The structure is: { "configVersion": N, "accounts": [...], ... }
    let storage_config: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|e| format!("Failed to parse extension storage JSON: {}", e))?;

    let raw_accounts = storage_config
        .get("accounts")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            "Extension storage does not contain an 'accounts' array".to_string()
        })?;

    // 3. Normalize into SiteAccount structs (reuses backup.rs logic)
    let mut accounts = backup::normalize_accounts(raw_accounts)?;

    if accounts.is_empty() {
        return Err("No accounts found in extension storage".to_string());
    }

    // 4. Fetch API keys for all accounts
    fetch_api_keys_for_accounts(&mut accounts).await;

    tracing::info!(
        count = accounts.len(),
        "Successfully synced accounts from browser extension"
    );

    Ok(accounts)
}

/// Fetch API Keys for all accounts using their access_tokens.
///
/// This calls `GET /api/token/` on each upstream to retrieve the actual `sk-xxx`
/// key needed for AI API calls. The access_token is only a management credential.
///
/// Errors are logged but non-fatal — accounts without keys can still be
/// imported; they just won't be usable for proxying until keys are fetched.
async fn fetch_api_keys_for_accounts(accounts: &mut [SiteAccount]) {
    populate_api_keys_for_accounts(
        accounts,
        std::time::Duration::from_secs(15),
        ApiKeyFetchScope::All,
    )
    .await;
}

#[tauri::command]
pub async fn load_config() -> Result<AppConfig, String> {
    Ok(config::load_app_config())
}

/// Refresh API Keys for all stored accounts.
///
/// Reads the current config, re-fetches API Keys from each upstream using
/// access_tokens, updates both `accounts` and `proxy_accounts`, persists
/// to disk, and returns a per-account summary.
#[tauri::command]
pub async fn refresh_api_keys(
    state: tauri::State<'_, ProxyServiceState>,
) -> Result<serde_json::Value, String> {
    let mut cfg = config::load_app_config();
    let existing_key_state: Vec<(String, bool)> = cfg
        .accounts
        .iter()
        .map(|account| {
            (
                account.id.clone(),
                has_usable_api_key(account.account_info.api_key.as_deref()),
            )
        })
        .collect();

    // Preserve already-selected usable keys; only fill missing or masked ones.
    populate_api_keys_for_accounts(
        &mut cfg.accounts,
        std::time::Duration::from_secs(15),
        ApiKeyFetchScope::MissingOrMasked,
    )
    .await;
    let cfg = config::normalized_app_config(&cfg);

    // Persist
    config::save_app_config(&cfg)?;
    sync_running_proxy_config(&state, &cfg).await;

    // Build summary
    let mut success = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut preserved = 0u32;
    let details: Vec<serde_json::Value> = cfg
        .accounts
        .iter()
        .map(|a| {
            let had_usable_key_before = existing_key_state
                .iter()
                .find(|(id, _)| id == &a.id)
                .map(|(_, had_key)| *had_key)
                .unwrap_or(false);
            let has_key = has_usable_api_key(a.account_info.api_key.as_deref());
            let is_sub2api = a.site_type == "sub2api";
            let api_key_status = if is_sub2api {
                "shared_access_token"
            } else if has_key {
                "usable"
            } else if a
                .account_info
                .api_key
                .as_deref()
                .map(|k| k.contains('*'))
                .unwrap_or(false)
            {
                "masked"
            } else {
                "missing"
            };
            if is_sub2api {
                skipped += 1;
            } else if had_usable_key_before {
                preserved += 1;
            } else if has_key {
                success += 1;
            } else {
                failed += 1;
            }
            serde_json::json!({
                "id": a.id,
                "site_name": a.site_name,
                "site_type": a.site_type,
                "has_api_key": has_key,
                "api_key_status": api_key_status,
                "api_key_preview": a.account_info.api_key.as_deref().map(|k| {
                    if k.len() > 12 {
                        format!("{}...{}", &k[..8], &k[k.len()-4..])
                    } else {
                        k.to_string()
                    }
                }),
            })
        })
        .collect();

    Ok(serde_json::json!({
        "success": success,
        "failed": failed,
        "skipped": skipped,
        "preserved": preserved,
        "total": cfg.accounts.len(),
        "accounts": details,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserLoginUpsertResult {
    pub action: String,
    pub profile_mode: String,
    pub account: SiteAccount,
    pub snapshot: browser_login::LoginExtractResult,
}

fn normalized_profile_mode(profile_mode: Option<&str>) -> &'static str {
    if profile_mode
        .map(|v| v.trim().eq_ignore_ascii_case("isolated"))
        .unwrap_or(false)
    {
        "isolated"
    } else {
        "main"
    }
}

fn as_bearer_token(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.to_ascii_lowercase().starts_with("bearer ") {
        trimmed[7..].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_site_url(site_url: &str) -> Result<String, String> {
    let raw = site_url.trim();
    if raw.is_empty() {
        return Err("Site URL is required".to_string());
    }

    let parsed = reqwest::Url::parse(raw)
        .map_err(|_| format!("Invalid site URL: {}", site_url))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err("Site URL must start with http:// or https://".to_string());
    }

    Ok(raw.trim_end_matches('/').to_string())
}

fn normalize_site_type(site_type: &str) -> String {
    let trimmed = site_type.trim();
    if trimmed.is_empty() {
        "new-api".to_string()
    } else {
        trimmed.to_string()
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn open_browser_login(
    site_url: String,
    profile_mode: Option<String>,
) -> Result<browser_login::LoginBrowserInfo, String> {
    let normalized_url = normalize_site_url(&site_url)?;
    let mode = normalized_profile_mode(profile_mode.as_deref());

    let data_dir = config::get_data_dir()?;
    let profile_dir = browser_login::resolve_profile_dir(&data_dir, mode)?;
    let debug_port = browser_login::pick_free_port()?;
    let chrome_path = browser_login::open_browser_for_login(&normalized_url, &profile_dir, debug_port)
        .await?
        .to_string_lossy()
        .to_string();

    Ok(browser_login::LoginBrowserInfo {
        site_url: normalized_url,
        debug_port,
        profile_mode: mode.to_string(),
        profile_path: profile_dir.to_string_lossy().to_string(),
        chrome_path,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn import_account_from_browser_login(
    site_url: String,
    site_type: String,
    debug_port: u16,
    profile_mode: Option<String>,
    site_name: Option<String>,
    disabled: Option<bool>,
) -> Result<BrowserLoginUpsertResult, String> {
    let normalized_url = normalize_site_url(&site_url)?;
    let normalized_type = normalize_site_type(&site_type);
    let mode = normalized_profile_mode(profile_mode.as_deref()).to_string();

    let snapshot = browser_login::extract_login_snapshot(&normalized_url, debug_port).await?;
    let user_id = snapshot
        .user_id
        .ok_or_else(|| "Login user id not detected. Please complete login and refresh the page.".to_string())?;
    let access_token = snapshot
        .access_token
        .clone()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            "Access token not found. Please complete login or generate token in site console first.".to_string()
        })?;
    let access_token = as_bearer_token(&access_token);

    let host_fallback = reqwest::Url::parse(&normalized_url)
        .ok()
        .and_then(|v| v.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "API Site".to_string());

    let provided_site_name = site_name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());

    let final_site_name = provided_site_name
        .or_else(|| snapshot.system_name.clone())
        .unwrap_or(host_fallback);

    let final_username = snapshot
        .username
        .clone()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| format!("user-{}", user_id));

    let now = now_millis();
    let mut cfg = config::load_app_config();

    let mut action = "created".to_string();
    let account_id: String;

    if let Some(existing) = cfg
        .accounts
        .iter_mut()
        .find(|a| a.site_url.trim_end_matches('/') == normalized_url && a.account_info.id == user_id)
    {
        existing.site_name = final_site_name.clone();
        existing.site_type = normalized_type.clone();
        existing.account_info.access_token = access_token.clone();
        existing.account_info.username = final_username.clone();
        existing.updated_at = now;
        existing.last_sync_time = now;
        existing.browser_profile_mode = Some(mode.clone());
        let data_dir = config::get_data_dir()?;
        let profile_dir = browser_login::resolve_profile_dir(&data_dir, &mode)?;
        existing.browser_profile_path = Some(profile_dir.to_string_lossy().to_string());
        if let Some(disabled_value) = disabled {
            existing.disabled = Some(disabled_value);
        }
        account_id = existing.id.clone();
        action = "updated".to_string();
    } else {
        let data_dir = config::get_data_dir()?;
        let profile_dir = browser_login::resolve_profile_dir(&data_dir, &mode)?;
        let new_account = SiteAccount {
            id: uuid::Uuid::new_v4().to_string(),
            site_name: final_site_name.clone(),
            site_url: normalized_url.clone(),
            site_type: normalized_type.clone(),
            account_info: crate::models::AccountInfo {
                id: user_id,
                access_token: access_token.clone(),
                api_key: None,
                username: final_username,
                quota: 0.0,
                today_prompt_tokens: 0,
                today_completion_tokens: 0,
                today_quota_consumption: 0.0,
                today_requests_count: 0,
                today_income: 0.0,
            },
            auth_type: "access_token".to_string(),
            last_sync_time: now,
            updated_at: now,
            created_at: now,
            notes: None,
            disabled: Some(disabled.unwrap_or(false)),
            health: None,
            exchange_rate: None,
            browser_profile_mode: Some(mode.clone()),
            browser_profile_path: Some(profile_dir.to_string_lossy().to_string()),
            proxy_health: None,
            proxy_priority: 0,
            proxy_weight: 10,
        };
        account_id = new_account.id.clone();
        cfg.accounts.push(new_account);
    }

    if let Ok(client) = hub_http_client(cfg.proxy.request_timeout) {
        if let Ok(api_key) = crate::proxy::key_fetcher::fetch_api_key(
            &client,
            &normalized_url,
            &normalized_type,
            &access_token,
            user_id,
        )
        .await
        {
            if let Some(account) = cfg.accounts.iter_mut().find(|a| a.id == account_id) {
                account.account_info.api_key = Some(api_key);
                account.updated_at = now_millis();
            }
        }
    }

    cfg.proxy_accounts = cfg
        .accounts
        .iter()
        .filter(|a| !a.disabled.unwrap_or(false))
        .cloned()
        .collect();

    config::save_app_config(&cfg)?;

    let account = cfg
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
        .ok_or_else(|| "Saved account not found".to_string())?;

    Ok(BrowserLoginUpsertResult {
        action,
        profile_mode: mode,
        account,
        snapshot,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn save_config(
    state: tauri::State<'_, ProxyServiceState>,
    config_data: AppConfig,
) -> Result<(), String> {
    let previous = config::load_app_config();
    let mut normalized = config::normalized_app_config(&config_data);
    auto_enable_accounts_with_new_manual_keys(&previous, &mut normalized);
    let normalized = config::normalized_app_config(&normalized);

    config::save_app_config(&normalized)?;
    desktop::sync_launch_on_startup(normalized.desktop.launch_on_startup)?;
    sync_running_proxy_config(&state, &normalized).await;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn proxy_start(
    state: tauri::State<'_, ProxyServiceState>,
    config_data: AppConfig,
) -> Result<(), String> {
    let config_data = config::normalized_app_config(&config_data);

    // Check if already running (drop guard before await)
    {
        let server = state.server.lock().await;
        if server.is_running() {
            return Err("Proxy is already running".to_string());
        }
    }

    // Start server (no lock held during await)
    let axum_server = crate::proxy::server::start_server(&config_data.proxy, &config_data.proxy_accounts)
        .await
        .map_err(|e| format!("Failed to start proxy: {}", e))?;

    // Capture monitor and token_manager references before moving the server
    let monitor_ref = axum_server.monitor.clone();
    let token_manager_ref = axum_server.token_manager.clone();
    let security_ref = axum_server.security.clone();

    // Set handle
    {
        let mut server = state.server.lock().await;
        server.set_server(axum_server);
    }

    // Store monitor for get_logs access
    {
        let mut monitor = state.monitor.write().await;
        *monitor = Some(monitor_ref);
    }

    // Store token_manager for get_available_models access
    {
        let mut tm = state.token_manager.write().await;
        *tm = Some(token_manager_ref);
    }
    {
        let mut security = state.security.write().await;
        *security = Some(security_ref);
    }

    // Persist config
    config::save_app_config(&config_data)?;

    Ok(())
}

#[tauri::command]
pub async fn proxy_stop(state: tauri::State<'_, ProxyServiceState>) -> Result<(), String> {
    let mut server = state.server.lock().await;

    if !server.is_running() {
        return Err("Proxy is not running".to_string());
    }

    server.stop().await;

    // Clear monitor reference
    {
        let mut monitor = state.monitor.write().await;
        *monitor = None;
    }

    // Clear token_manager reference
    {
        let mut tm = state.token_manager.write().await;
        *tm = None;
    }
    {
        let mut security = state.security.write().await;
        *security = None;
    }

    Ok(())
}

#[tauri::command]
pub async fn get_proxy_status(
    state: tauri::State<'_, ProxyServiceState>,
) -> Result<serde_json::Value, String> {
    let server = state.server.lock().await;

    Ok(serde_json::json!({
        "running": server.is_running(),
    }))
}

#[tauri::command]
pub async fn get_logs(
    state: tauri::State<'_, ProxyServiceState>,
) -> Result<serde_json::Value, String> {
    refresh_site_price_cache_if_needed().await;
    refresh_price_cache_if_needed().await;
    let monitor = state.monitor.read().await;

    if let Some(ref mon) = *monitor {
        let logs: Vec<Value> = mon
            .get_logs(0, 100)
            .into_iter()
            .map(|mut log| {
                log.estimated_cost = crate::proxy::middleware::monitor::recompute_estimated_cost_from_log(&log);
                serde_json::to_value(log).unwrap_or(Value::Null)
            })
            .collect();
        Ok(serde_json::json!({
            "total": mon.get_count(),
            "logs": logs,
        }))
    } else {
        Ok(serde_json::json!({
            "total": 0,
            "logs": [],
        }))
    }
}

#[tauri::command]
pub async fn replay_request(
    state: tauri::State<'_, ProxyServiceState>,
    log_id: Option<String>,
    #[allow(non_snake_case)] logId: Option<String>,
) -> Result<serde_json::Value, String> {
    let log_id = log_id
        .or(logId)
        .ok_or_else(|| "Missing required parameter: log_id/logId".to_string())?;

    let monitor = state.monitor.read().await;

    let mon = monitor.as_ref().ok_or("Proxy not running")?;

    let log = mon
        .get_log(&log_id)
        .ok_or_else(|| format!("Log {} not found", log_id))?;

    let body = log
        .original_request_body
        .or(log.request_body)
        .ok_or("No request body captured for this log")?;

    // Read config to get port and API key for auth
    let config = crate::modules::config::load_app_config();
    let port = config.proxy.port;
    let api_key = &config.proxy.api_key;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.proxy.request_timeout))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let url = format!("http://127.0.0.1:{}{}", port, log.url);

    let resp = client
        .request(
            reqwest::Method::from_bytes(log.method.as_bytes())
                .unwrap_or(reqwest::Method::POST),
            &url,
        )
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Replay request failed: {}", e))?;

    let status = resp.status().as_u16();
    let resp_body = resp
        .text()
        .await
        .unwrap_or_else(|e| format!("Failed to read response: {}", e));

    Ok(serde_json::json!({
        "status": status,
        "body": resp_body,
    }))
}

#[tauri::command]
pub async fn get_available_models(
    state: tauri::State<'_, ProxyServiceState>,
) -> Result<Vec<String>, String> {
    let cache = crate::proxy::model_cache::global();

    // 1. Fast path: proxy running and has model data in memory
    {
        let tm = state.token_manager.read().await;
        if let Some(ref token_manager) = *tm {
            let models = token_manager.get_all_models().await;
            if !models.is_empty() {
                tracing::debug!(count = models.len(), "Returning models from proxy registry");
                return Ok(models);
            }
        }
    }

    // 2. Fast path: file cache already loaded in memory
    {
        let models = cache.get_all_models().await;
        if !models.is_empty() {
            tracing::debug!(count = models.len(), "Returning models from memory cache");
            return Ok(models);
        }
    }

    // 3. Try loading from disk (first call after startup)
    cache.load_from_disk().await;
    {
        let models = cache.get_all_models().await;
        if !models.is_empty() {
            tracing::info!(count = models.len(), "Returning models from disk cache");
            // Also feed into proxy registry if running
            populate_proxy_registry_from_cache(&state, &cache).await;
            return Ok(models);
        }
    }

    // 4. Slow path: first-ever launch — fetch from upstreams (once only)
    let _guard = match cache.try_acquire_fetch_guard() {
        Some(g) => g,
        None => {
            // Another call is already fetching; wait briefly then return whatever is available
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            return Ok(cache.get_all_models().await);
        }
    };

    // Double-check after acquiring guard (another call may have populated)
    {
        let models = cache.get_all_models().await;
        if !models.is_empty() {
            return Ok(models);
        }
    }

    tracing::info!("No model cache found — fetching from upstreams (first time)");

    let cfg = config::load_app_config();
    let accounts: Vec<&SiteAccount> = cfg
        .proxy_accounts
        .iter()
        .filter(|a| !a.disabled.unwrap_or(false) && !a.account_info.access_token.is_empty())
        .collect();

    if accounts.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let mut handles = Vec::with_capacity(accounts.len());
    for account in &accounts {
        let client = client.clone();
        let site_url = account.site_url.clone();
        let access_token = as_bearer_token(&account.account_info.access_token);
        let api_key = account.account_info.api_key.clone();
        let account_id = account.id.clone();
        let site_type = account.site_type.clone();
        let user_id = Some(account.account_info.id);

        handles.push(tokio::spawn(async move {
            let models = fetch_models_for_account(
                &client,
                &site_url,
                &access_token,
                api_key.as_deref(),
                &account_id,
                &site_type,
                user_id,
            )
            .await;
            (account_id, models)
        }));
    }

    let mut all_models = HashSet::new();
    for h in handles {
        if let Ok((account_id, models)) = h.await {
            if !models.is_empty() {
                let set: HashSet<String> = models.iter().cloned().collect();
                cache.set_account_models(&account_id, set).await;
                for m in models {
                    all_models.insert(m);
                }
            }
        }
    }

    // Persist to disk
    cache.save_to_disk();

    let mut sorted: Vec<String> = all_models.into_iter().collect();
    sorted.sort();

    tracing::info!(total_models = sorted.len(), "Initial model fetch complete — cached to disk");

    // Feed into proxy registry if running
    populate_proxy_registry_from_cache(&state, &cache).await;

    Ok(sorted)
}

#[tauri::command]
pub async fn get_proxy_stats() -> Result<serde_json::Value, String> {
    let stats = crate::proxy::proxy_stats::global().get_stats();
    serde_json::to_value(&stats).map_err(|e| format!("Failed to serialize stats: {}", e))
}

#[tauri::command]
pub async fn get_proxy_stats_view(scope: Option<String>) -> Result<serde_json::Value, String> {
    refresh_site_price_cache_if_needed().await;
    refresh_price_cache_if_needed().await;
    let scope = scope
        .as_deref()
        .and_then(StatsScope::parse)
        .unwrap_or(StatsScope::Daily);
    let stats = crate::proxy::proxy_stats::global().get_scoped_stats(scope);
    serde_json::to_value(&stats).map_err(|e| format!("Failed to serialize scoped stats: {}", e))
}

#[tauri::command]
pub async fn get_token_stats_view(scope: Option<String>) -> Result<serde_json::Value, String> {
    refresh_site_price_cache_if_needed().await;
    refresh_price_cache_if_needed().await;
    let scope = scope
        .as_deref()
        .and_then(StatsScope::parse)
        .unwrap_or(StatsScope::Daily);
    let stats = crate::proxy::proxy_stats::global().get_token_stats_view(scope);
    serde_json::to_value(&stats).map_err(|e| format!("Failed to serialize token stats: {}", e))
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyModelPriceQuote {
    pub billing_mode: crate::proxy::site_price_cache::SiteModelBillingMode,
    pub source_count: usize,
    pub from_site_pricing: bool,
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub input_per_million_max: Option<f64>,
    pub output_per_million_max: Option<f64>,
    pub request_price: Option<f64>,
    pub request_price_max: Option<f64>,
}

#[tauri::command]
pub async fn get_proxy_model_catalog(
    state: tauri::State<'_, ProxyServiceState>,
) -> Result<Vec<AccountModels>, String> {
    {
        let tm = state.token_manager.read().await;
        if let Some(ref token_manager) = *tm {
            let rows = token_manager.get_models_by_account().await;
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
    }

    let cache = crate::proxy::model_cache::global();
    cache.load_from_disk().await;

    let cfg = config::load_app_config();
    let tm = TokenManager::new();
    tm.load_from_accounts(&cfg.proxy_accounts);
    tm.load_models_from_cache(&cache).await;

    Ok(tm.get_models_by_account().await)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_proxy_model_prices(
    models: Vec<String>,
    account_ids: Option<Vec<String>>,
) -> Result<HashMap<String, ProxyModelPriceQuote>, String> {
    refresh_site_price_cache_if_needed().await;
    refresh_price_cache_if_needed().await;

    let site_cache = crate::proxy::site_price_cache::global();
    let generic_cache = crate::proxy::price_cache::global();
    let account_ids = account_ids.unwrap_or_default();
    let mut prices = HashMap::new();
    for model in models {
        let key = model.trim();
        if key.is_empty() || prices.contains_key(key) {
            continue;
        }
        if let Some(price) = site_cache.quote_model(&account_ids, key) {
            prices.insert(
                key.to_string(),
                ProxyModelPriceQuote {
                    billing_mode: price.billing_mode,
                    source_count: price.source_count,
                    from_site_pricing: price.from_site_pricing,
                    input_per_million: price.input_per_million,
                    output_per_million: price.output_per_million,
                    input_per_million_max: price.input_per_million_max,
                    output_per_million_max: price.output_per_million_max,
                    request_price: price.request_price,
                    request_price_max: price.request_price_max,
                },
            );
            continue;
        }

        if let Some(price) = generic_cache.get_price(key) {
            prices.insert(
                key.to_string(),
                ProxyModelPriceQuote {
                    billing_mode: crate::proxy::site_price_cache::SiteModelBillingMode::Tokens,
                    source_count: 0,
                    from_site_pricing: false,
                    input_per_million: Some(price.input_cost_per_token * 1_000_000.0),
                    output_per_million: Some(price.output_cost_per_token * 1_000_000.0),
                    input_per_million_max: Some(price.input_cost_per_token * 1_000_000.0),
                    output_per_million_max: Some(price.output_cost_per_token * 1_000_000.0),
                    request_price: None,
                    request_price_max: None,
                },
            );
        }
    }

    Ok(prices)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn refresh_price_cache_if_needed() {
    let cache = crate::proxy::price_cache::global();
    if cache.is_empty() || cache.needs_refresh() {
        cache.refresh().await;
    }
}

async fn refresh_site_price_cache_if_needed() {
    let cfg = config::load_app_config();
    if cfg.proxy_accounts.is_empty() {
        return;
    }

    let cache = crate::proxy::site_price_cache::global();
    let pending_accounts: Vec<SiteAccount> = cfg
        .proxy_accounts
        .into_iter()
        .filter(|account| cache.needs_refresh(&account.id))
        .collect();
    if pending_accounts.is_empty() {
        return;
    }

    let Ok(client) = hub_http_client(cfg.proxy.request_timeout) else {
        return;
    };

    let mut handles = Vec::with_capacity(pending_accounts.len());
    for account in pending_accounts {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let result = hub_service::fetch_model_pricing(&client, &account).await;
            (account.id, account.site_name, result)
        }));
    }

    for handle in handles {
        match handle.await {
            Ok((account_id, _site_name, Ok(payload))) => {
                cache.set_account_pricing(&account_id, &payload);
            }
            Ok((account_id, site_name, Err(error))) => {
                tracing::debug!(
                    account_id = %account_id,
                    site_name = %site_name,
                    error = %error,
                    "Failed to refresh site pricing cache"
                );
            }
            Err(error) => {
                tracing::debug!(error = %error, "Site pricing refresh task failed");
            }
        }
    }
}

fn hub_http_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs.max(10)))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

fn find_account_by_id(cfg: &AppConfig, account_id: &str) -> Result<SiteAccount, String> {
    cfg.accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
        .ok_or_else(|| format!("Account not found: {}", account_id))
}

fn apply_detection_snapshot_to_account(
    account: &mut SiteAccount,
    detection: &crate::modules::hub_service::HubDetectionResult,
    timestamp_ms: i64,
) {
    if let Some(balance) = detection.balance {
        account.account_info.quota = balance;
    }
    if let Some(today_usage) = detection.today_usage {
        account.account_info.today_quota_consumption = today_usage;
    }
    if let Some(prompt_tokens) = detection.today_prompt_tokens {
        account.account_info.today_prompt_tokens = prompt_tokens;
    }
    if let Some(completion_tokens) = detection.today_completion_tokens {
        account.account_info.today_completion_tokens = completion_tokens;
    }
    if let Some(requests) = detection.today_requests_count {
        account.account_info.today_requests_count = requests;
    }

    account.last_sync_time = timestamp_ms;
    account.updated_at = timestamp_ms;

    account.health = Some(crate::models::HealthStatus {
        status: if detection.status == "success" {
            "normal".to_string()
        } else {
            "error".to_string()
        },
        reason: if detection.status == "success" {
            None
        } else {
            detection.error.clone()
        },
    });
}

fn apply_balance_snapshot_to_account(
    account: &mut SiteAccount,
    snapshot: &crate::modules::hub_service::BalanceSnapshot,
    timestamp_ms: i64,
) {
    if let Some(balance) = snapshot.balance {
        account.account_info.quota = balance;
    }
    if let Some(today_usage) = snapshot.today_usage {
        account.account_info.today_quota_consumption = today_usage;
    }
    if let Some(prompt_tokens) = snapshot.today_prompt_tokens {
        account.account_info.today_prompt_tokens = prompt_tokens;
    }
    if let Some(completion_tokens) = snapshot.today_completion_tokens {
        account.account_info.today_completion_tokens = completion_tokens;
    }
    if let Some(requests) = snapshot.today_requests_count {
        account.account_info.today_requests_count = requests;
    }

    account.last_sync_time = timestamp_ms;
    account.updated_at = timestamp_ms;
}

fn persist_detection_to_config(
    cfg: &mut AppConfig,
    detection: &crate::modules::hub_service::HubDetectionResult,
) {
    let ts = now_millis();

    for account in &mut cfg.accounts {
        if account.id == detection.account_id {
            apply_detection_snapshot_to_account(account, detection, ts);
        }
    }
    for account in &mut cfg.proxy_accounts {
        if account.id == detection.account_id {
            apply_detection_snapshot_to_account(account, detection, ts);
        }
    }
}

#[tauri::command]
pub async fn list_hub_accounts() -> Result<Vec<SiteAccount>, String> {
    let cfg = config::load_app_config();
    Ok(cfg.accounts)
}

#[derive(Debug, Clone, Serialize)]
pub struct HubBalanceRefreshResponse {
    pub accounts: Vec<SiteAccount>,
    pub refreshed: usize,
    pub failed: usize,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn refresh_hub_balances() -> Result<HubBalanceRefreshResponse, String> {
    let mut cfg = config::load_app_config();
    if cfg.accounts.is_empty() {
        return Ok(HubBalanceRefreshResponse {
            accounts: Vec::new(),
            refreshed: 0,
            failed: 0,
        });
    }

    let client = hub_http_client(cfg.proxy.request_timeout)?;
    let accounts = cfg.accounts.clone();
    let mut handles = Vec::with_capacity(accounts.len());

    for account in accounts {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let result = hub_service::fetch_balance_overview(&client, &account).await;
            (account.id, result)
        }));
    }

    let timestamp_ms = now_millis();
    let mut refreshed = 0usize;
    let mut failed = 0usize;

    for handle in handles {
        let (account_id, result) = handle
            .await
            .map_err(|e| format!("Refresh balance task failed: {}", e))?;

        match result {
            Ok(snapshot) => {
                refreshed += 1;
                for account in &mut cfg.accounts {
                    if account.id == account_id {
                        apply_balance_snapshot_to_account(account, &snapshot, timestamp_ms);
                    }
                }
                for account in &mut cfg.proxy_accounts {
                    if account.id == account_id {
                        apply_balance_snapshot_to_account(account, &snapshot, timestamp_ms);
                    }
                }
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    account_id = %account_id,
                    error = %error,
                    "Failed to refresh hub balance snapshot"
                );
            }
        }
    }

    config::save_app_config(&cfg)?;

    Ok(HubBalanceRefreshResponse {
        accounts: cfg.accounts,
        refreshed,
        failed,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn refresh_selected_hub_balances(
    account_ids: Vec<String>,
) -> Result<HubBalanceRefreshResponse, String> {
    let mut cfg = config::load_app_config();
    if cfg.accounts.is_empty() || account_ids.is_empty() {
        return Ok(HubBalanceRefreshResponse {
            accounts: cfg.accounts,
            refreshed: 0,
            failed: 0,
        });
    }

    let requested_ids: std::collections::HashSet<String> = account_ids.into_iter().collect();
    let client = hub_http_client(cfg.proxy.request_timeout)?;
    let accounts: Vec<SiteAccount> = cfg
        .accounts
        .iter()
        .filter(|account| requested_ids.contains(&account.id))
        .cloned()
        .collect();

    let mut handles = Vec::with_capacity(accounts.len());
    for account in accounts {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            let result = hub_service::fetch_balance_overview(&client, &account).await;
            (account.id, result)
        }));
    }

    let timestamp_ms = now_millis();
    let mut refreshed = 0usize;
    let mut failed = 0usize;

    for handle in handles {
        let (account_id, result) = handle
            .await
            .map_err(|e| format!("Refresh balance task failed: {}", e))?;

        match result {
            Ok(snapshot) => {
                refreshed += 1;
                for account in &mut cfg.accounts {
                    if account.id == account_id {
                        apply_balance_snapshot_to_account(account, &snapshot, timestamp_ms);
                    }
                }
                for account in &mut cfg.proxy_accounts {
                    if account.id == account_id {
                        apply_balance_snapshot_to_account(account, &snapshot, timestamp_ms);
                    }
                }
            }
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    account_id = %account_id,
                    error = %error,
                    "Failed to refresh selected hub balance snapshot"
                );
            }
        }
    }

    config::save_app_config(&cfg)?;

    Ok(HubBalanceRefreshResponse {
        accounts: cfg.accounts,
        refreshed,
        failed,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn detect_hub_account(
    account_id: String,
    include_details: Option<bool>,
) -> Result<crate::modules::hub_service::HubDetectionResult, String> {
    let mut cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;

    let client = hub_http_client(cfg.proxy.request_timeout)?;
    let detection =
        hub_service::detect_account(&client, &account, include_details.unwrap_or(true)).await;

    persist_detection_to_config(&mut cfg, &detection);
    config::save_app_config(&cfg)?;

    Ok(detection)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn detect_all_hub_accounts(
    include_details: Option<bool>,
) -> Result<Vec<crate::modules::hub_service::HubDetectionResult>, String> {
    let mut cfg = config::load_app_config();
    if cfg.accounts.is_empty() {
        return Ok(Vec::new());
    }

    let include = include_details.unwrap_or(true);
    let timeout = cfg.proxy.request_timeout;
    let client = hub_http_client(timeout)?;
    let accounts = cfg.accounts.clone();

    let mut handles = Vec::with_capacity(accounts.len());
    for account in accounts {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            hub_service::detect_account(&client, &account, include).await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => return Err(format!("Detect task failed: {}", e)),
        }
    }

    for result in &results {
        persist_detection_to_config(&mut cfg, result);
    }
    config::save_app_config(&cfg)?;

    Ok(results)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn hub_checkin_account(
    account_id: String,
) -> Result<crate::modules::hub_service::HubCheckinResponse, String> {
    let mut cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;

    let client = hub_http_client(cfg.proxy.request_timeout)?;
    let checkin = hub_service::checkin_account(&client, &account).await;

    let detection = if checkin.success {
        let result = hub_service::detect_account(&client, &account, true).await;
        persist_detection_to_config(&mut cfg, &result);
        Some(result)
    } else {
        None
    };

    if detection.is_some() {
        config::save_app_config(&cfg)?;
    }

    Ok(crate::modules::hub_service::HubCheckinResponse {
        checkin,
        detection,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn hub_fetch_api_tokens(account_id: String) -> Result<Vec<Value>, String> {
    let cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;
    let client = hub_http_client(cfg.proxy.request_timeout)?;
    hub_service::list_api_tokens(&client, &account).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn hub_create_api_token(
    account_id: String,
    token_data: Value,
) -> Result<Vec<Value>, String> {
    let cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;
    let client = hub_http_client(cfg.proxy.request_timeout)?;
    hub_service::create_api_token(&client, &account, &token_data).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn hub_delete_api_token(
    account_id: String,
    token_identifier: Value,
) -> Result<Vec<Value>, String> {
    let cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;
    let client = hub_http_client(cfg.proxy.request_timeout)?;
    hub_service::delete_api_token(&client, &account, &token_identifier).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn hub_fetch_user_groups(account_id: String) -> Result<Value, String> {
    let cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;
    let client = hub_http_client(cfg.proxy.request_timeout)?;
    hub_service::fetch_user_groups(&client, &account).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn hub_fetch_model_pricing(account_id: String) -> Result<Value, String> {
    let cfg = config::load_app_config();
    let account = find_account_by_id(&cfg, &account_id)?;
    let client = hub_http_client(cfg.proxy.request_timeout)?;
    hub_service::fetch_model_pricing(&client, &account).await
}

/// If proxy is running and its model registry is empty, populate it from the file cache.
async fn populate_proxy_registry_from_cache(
    state: &tauri::State<'_, ProxyServiceState>,
    cache: &crate::proxy::model_cache::ModelCache,
) {
    let tm = state.token_manager.read().await;
    if let Some(ref token_manager) = *tm {
        if token_manager.get_all_models().await.is_empty() {
            token_manager.load_models_from_cache(cache).await;
        }
    }
}

/// Fetch model list from a single account, using the correct endpoint per site type:
///   - new-api / one-api / Veloera / etc. → `/api/user/models` (returns `string[]` in `data`)
///   - one-hub / done-hub → `/api/available_model` (returns `{model_name: {...}}` map)
///   - sub2api → not supported (no model listing endpoint)
///   - fallback → `/v1/models` (OpenAI-compatible, needs API key)
async fn fetch_models_for_account(
    client: &reqwest::Client,
    site_url: &str,
    access_token: &str,
    api_key: Option<&str>,
    account_id: &str,
    site_type: &str,
    user_id: Option<i64>,
) -> Vec<String> {
    let base = site_url.trim_end_matches('/');

    match site_type {
        "sub2api" => {
            // Sub2API has no model listing endpoint
            Vec::new()
        }
        "one-hub" | "done-hub" => {
            // OneHub/DoneHub: /api/available_model returns {model_name: {details}}
            fetch_models_onehub(client, base, access_token, account_id, user_id).await
        }
        _ => {
            // New-API / One-API / Veloera family: try /api/user/models first
            let models = fetch_models_newapi(client, base, access_token, account_id, user_id).await;
            if !models.is_empty() {
                return models;
            }
            // Fallback: /v1/models (works for accounts with sk- API keys)
            let credential = api_key
                .map(str::trim)
                .filter(|value| has_usable_api_key(Some(*value)))
                .unwrap_or(access_token);
            fetch_models_openai(client, base, credential, account_id, user_id).await
        }
    }
}

async fn sync_running_proxy_accounts(
    state: &tauri::State<'_, ProxyServiceState>,
    accounts: &[SiteAccount],
) {
    let maybe_tm = { state.token_manager.read().await.clone() };
    let Some(token_manager) = maybe_tm else {
        return;
    };

    token_manager.load_from_accounts(accounts);

    let cache = crate::proxy::model_cache::global();
    cache.load_from_disk().await;
    if !cache.is_empty() {
        token_manager.load_models_from_cache(&cache).await;
    }

    tracing::info!(
        account_count = accounts.len(),
        "Running proxy token pool refreshed from updated config"
    );
}

async fn sync_running_proxy_config(
    state: &tauri::State<'_, ProxyServiceState>,
    config_data: &AppConfig,
) {
    sync_running_proxy_accounts(state, &config_data.proxy_accounts).await;

    let maybe_monitor = { state.monitor.read().await.clone() };
    if let Some(monitor) = maybe_monitor {
        monitor.set_enabled(config_data.proxy.enable_logging);
    }

    let maybe_security = { state.security.read().await.clone() };
    if let Some(security) = maybe_security {
        let mut guard = security.write().await;
        guard.auth_mode = config_data.proxy.auth_mode.clone();
        guard.api_key = config_data.proxy.api_key.clone();
        guard.admin_password = config_data.proxy.admin_password.clone();
        guard.api_keys = config_data.proxy.api_keys.clone();
    }
}

fn auto_enable_accounts_with_new_manual_keys(previous: &AppConfig, next: &mut AppConfig) {
    let previous_accounts: std::collections::HashMap<&str, &SiteAccount> = previous
        .accounts
        .iter()
        .map(|account| (account.id.as_str(), account))
        .collect();

    for account in &mut next.accounts {
        if !account.disabled.unwrap_or(false) {
            continue;
        }

        let Some(previous_account) = previous_accounts.get(account.id.as_str()) else {
            continue;
        };

        let previous_key = previous_account.account_info.api_key.as_deref().map(str::trim);
        let next_key = account.account_info.api_key.as_deref().map(str::trim);
        let changed = previous_key != next_key;
        let became_usable = has_usable_api_key(next_key) && !has_usable_api_key(previous_key);

        if changed && became_usable {
            account.disabled = Some(false);
            tracing::info!(
                account_id = %account.id,
                site_name = %account.site_name,
                "Auto-enabled account after manual API key update"
            );
        }
    }
}

fn build_user_id_headers(user_id: Option<i64>) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if let Some(id) = user_id {
        if id > 0 {
            let id_str = id.to_string();
            for name in [
                "New-API-User",
                "Veloera-User",
                "voapi-user",
                "User-id",
                "Rix-Api-User",
                "neo-api-user",
            ] {
                headers.push((name.to_string(), id_str.clone()));
            }
        }
    }
    headers
}

/// New-API family: GET /api/user/models → { "data": ["model-a", "model-b", ...] }
async fn fetch_models_newapi(
    client: &reqwest::Client,
    base: &str,
    access_token: &str,
    account_id: &str,
    user_id: Option<i64>,
) -> Vec<String> {
    let url = format!("{}/api/user/models", base);

    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token));
    for (k, v) in build_user_id_headers(user_id) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                // Response shape: { "success": true, "data": ["model-a", "model-b"] }
                if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
                    let models: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    if !models.is_empty() {
                        tracing::info!(account_id, model_count = models.len(), "Fetched models via /api/user/models");
                        return models;
                    }
                }
            }
        }
        Ok(r) => {
            tracing::info!(account_id, status = r.status().as_u16(), "/api/user/models failed, will try fallback");
        }
        Err(e) => {
            tracing::warn!(account_id, error = %e, "Failed to connect for /api/user/models");
        }
    }

    Vec::new()
}

/// OneHub/DoneHub: GET /api/available_model → { "data": { "model-name": {...}, ... } }
async fn fetch_models_onehub(
    client: &reqwest::Client,
    base: &str,
    access_token: &str,
    account_id: &str,
    user_id: Option<i64>,
) -> Vec<String> {
    let url = format!("{}/api/available_model", base);

    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token));
    for (k, v) in build_user_id_headers(user_id) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                // Response shape: { "data": { "gpt-4": {...}, "claude-3": {...} } }
                if let Some(obj) = body.get("data").and_then(|d| d.as_object()) {
                    let models: Vec<String> = obj.keys().cloned().collect();
                    if !models.is_empty() {
                        tracing::info!(account_id, model_count = models.len(), "Fetched models via /api/available_model");
                        return models;
                    }
                }
            }
        }
        Ok(r) => {
            tracing::warn!(account_id, status = r.status().as_u16(), "/api/available_model failed");
        }
        Err(e) => {
            tracing::warn!(account_id, error = %e, "Failed to connect for /api/available_model");
        }
    }

    Vec::new()
}

/// Validate an API key by hitting GET /v1/models on the upstream.
///
/// Returns a JSON object: { "valid": bool, "model_count": u32, "error": string|null }
#[tauri::command(rename_all = "snake_case")]
pub async fn validate_api_key(
    site_url: String,
    api_key: String,
    site_type: String,
) -> Result<serde_json::Value, String> {
    let base = site_url.trim_end_matches('/');

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Pick the right endpoint per site type
    let url = match site_type.as_str() {
        "one-hub" | "done-hub" => format!("{}/api/available_model", base),
        _ => format!("{}/v1/models", base),
    };

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            if status == 401 || status == 403 {
                return Ok(serde_json::json!({
                    "valid": false,
                    "model_count": 0,
                    "error": format!("Authentication failed (HTTP {})", status),
                }));
            }
            if !r.status().is_success() {
                return Ok(serde_json::json!({
                    "valid": false,
                    "model_count": 0,
                    "error": format!("Upstream returned HTTP {}", status),
                }));
            }

            // Try to parse model list from response
            let model_count = if let Ok(body) = r.json::<serde_json::Value>().await {
                if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
                    arr.len()
                } else if let Some(obj) = body.get("data").and_then(|d| d.as_object()) {
                    obj.len()
                } else {
                    0
                }
            } else {
                0
            };

            Ok(serde_json::json!({
                "valid": true,
                "model_count": model_count,
                "error": null,
            }))
        }
        Err(e) => {
            let msg = if e.is_timeout() {
                "Connection timed out".to_string()
            } else if e.is_connect() {
                "Failed to connect to upstream".to_string()
            } else {
                format!("Request failed: {}", e)
            };
            Ok(serde_json::json!({
                "valid": false,
                "model_count": 0,
                "error": msg,
            }))
        }
    }
}

/// OpenAI-compatible: GET /v1/models → { "data": [{"id": "model-name"}, ...] }
async fn fetch_models_openai(
    client: &reqwest::Client,
    base: &str,
    access_token: &str,
    account_id: &str,
    user_id: Option<i64>,
) -> Vec<String> {
    let url = format!("{}/v1/models", base);

    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token));
    for (k, v) in build_user_id_headers(user_id) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await;

    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
                    let models: Vec<String> = arr
                        .iter()
                        .filter_map(|item| item.get("id").and_then(|id| id.as_str()).map(String::from))
                        .collect();
                    if !models.is_empty() {
                        tracing::info!(account_id, model_count = models.len(), "Fetched models via /v1/models");
                        return models;
                    }
                }
            }
        }
        Ok(r) => {
            let status = r.status().as_u16();
            if status != 401 && status != 403 {
                tracing::warn!(account_id, status, "/v1/models failed");
            }
        }
        Err(e) => {
            tracing::warn!(account_id, error = %e, "Failed to connect for /v1/models");
        }
    }

    Vec::new()
}
