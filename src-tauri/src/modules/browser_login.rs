use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginBrowserInfo {
    pub site_url: String,
    pub debug_port: u16,
    pub profile_mode: String,
    pub profile_path: String,
    pub chrome_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginExtractResult {
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub system_name: Option<String>,
    pub access_token: Option<String>,
    pub supports_checkin: Option<bool>,
    pub can_checkin: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CdpTarget {
    #[serde(rename = "type")]
    target_type: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    ws_url: Option<String>,
}

const LOCAL_STORAGE_SNAPSHOT_SCRIPT: &str = r#"
(() => {
  const safeParse = (raw) => {
    if (!raw) return null;
    try { return JSON.parse(raw); } catch (_) { return null; }
  };
  const pickObj = (...objs) => {
    for (const obj of objs) {
      if (obj && typeof obj === 'object') return obj;
    }
    return {};
  };
  const toNum = (value) => {
    if (typeof value === 'number' && Number.isFinite(value)) return value;
    if (typeof value === 'string' && value.trim()) {
      const parsed = Number(value);
      if (Number.isFinite(parsed)) return parsed;
    }
    return null;
  };
  const firstNonEmpty = (...values) => {
    for (const value of values) {
      if (typeof value === 'string' && value.trim().length > 0) {
        return value.trim();
      }
    }
    return '';
  };

  const user = pickObj(
    safeParse(localStorage.getItem('user')),
    safeParse(localStorage.getItem('userInfo'))
  );
  const siteInfo = pickObj(
    safeParse(localStorage.getItem('siteInfo')),
    safeParse(localStorage.getItem('siteConfig'))
  );
  const auth = pickObj(
    safeParse(localStorage.getItem('auth')),
    safeParse(localStorage.getItem('authentication'))
  );
  const status = pickObj(
    safeParse(localStorage.getItem('status')),
    safeParse(localStorage.getItem('siteStatus'))
  );
  const checkin = pickObj(
    safeParse(localStorage.getItem('checkin')),
    safeParse(localStorage.getItem('checkIn')),
    safeParse(localStorage.getItem('check_in'))
  );

  const userId =
    toNum(user.id) ??
    toNum(user.user_id) ??
    toNum(user.userId) ??
    toNum(siteInfo.id) ??
    toNum(siteInfo.user_id) ??
    toNum(siteInfo.userId) ??
    toNum(localStorage.getItem('user_id')) ??
    toNum(localStorage.getItem('userId')) ??
    toNum(localStorage.getItem('uid'));

  const username = firstNonEmpty(
    user.username,
    user.name,
    user.display_name,
    user.displayName,
    user.nickname,
    siteInfo.username,
    siteInfo.name,
    localStorage.getItem('username'),
    localStorage.getItem('nickname')
  );

  const systemName = firstNonEmpty(
    siteInfo.system_name,
    siteInfo.systemName,
    siteInfo.site_name,
    siteInfo.siteName,
    localStorage.getItem('system_name'),
    localStorage.getItem('systemName'),
    localStorage.getItem('site_name'),
    localStorage.getItem('siteName')
  );

  const accessToken = firstNonEmpty(
    user.access_token,
    user.accessToken,
    user.token,
    siteInfo.access_token,
    siteInfo.accessToken,
    siteInfo.token,
    auth.access_token,
    auth.accessToken,
    auth.token,
    localStorage.getItem('access_token'),
    localStorage.getItem('accessToken'),
    localStorage.getItem('token'),
    localStorage.getItem('auth_token'),
    localStorage.getItem('authToken')
  );

  const supportsCheckInRaw =
    (typeof siteInfo.check_in_enabled === 'boolean' ? siteInfo.check_in_enabled : undefined) ??
    (typeof siteInfo.checkin_enabled === 'boolean' ? siteInfo.checkin_enabled : undefined) ??
    (typeof status.check_in_enabled === 'boolean' ? status.check_in_enabled : undefined) ??
    (typeof status.checkin_enabled === 'boolean' ? status.checkin_enabled : undefined) ??
    (typeof checkin.enabled === 'boolean' ? checkin.enabled : undefined);

  const canCheckInRaw =
    (typeof user.can_check_in === 'boolean' ? user.can_check_in : undefined) ??
    (typeof checkin.can_check_in === 'boolean' ? checkin.can_check_in : undefined) ??
    (checkin.stats && typeof checkin.stats.checked_in_today === 'boolean'
      ? !checkin.stats.checked_in_today
      : undefined);

  return {
    user_id: userId,
    username,
    system_name: systemName,
    access_token: accessToken,
    supports_checkin: supportsCheckInRaw,
    can_checkin: canCheckInRaw,
    current_url: window.location.href,
  };
})()
"#;

fn normalize_profile_mode(profile_mode: &str) -> &'static str {
    if profile_mode.trim().eq_ignore_ascii_case("isolated") {
        "isolated"
    } else {
        "main"
    }
}

fn build_create_token_script(user_id: i64) -> String {
    format!(
        r#"
(async () => {{
  try {{
    const userId = {user_id};
    const headers = {{
      "Content-Type": "application/json",
      "New-API-User": String(userId),
      "Veloera-User": String(userId),
      "voapi-user": String(userId),
      "User-id": String(userId),
      "Rix-Api-User": String(userId),
      "neo-api-user": String(userId),
      "Cache-Control": "no-store",
      "Pragma": "no-cache"
    }};
    const tokenUrl = new URL("/api/user/token", window.location.origin).toString();
    const response = await fetch(tokenUrl, {{
      method: "GET",
      credentials: "include",
      headers,
    }});
    const text = await response.text();
    let payload = null;
    try {{
      payload = JSON.parse(text);
    }} catch (_err) {{}}

    if (!response.ok) {{
      return {{
        ok: false,
        error: payload?.message || text.slice(0, 200) || `HTTP ${{response.status}}`,
      }};
    }}

    if (!payload || payload.success !== true || !payload.data) {{
      return {{
        ok: false,
        error: payload?.message || "Token endpoint returned invalid payload",
      }};
    }}

    return {{ ok: true, token: String(payload.data) }};
  }} catch (error) {{
    return {{ ok: false, error: String(error) }};
  }}
}})()
"#
    )
}

fn normalize_site_url(site_url: &str) -> Result<String, String> {
    let raw = site_url.trim();
    if raw.is_empty() {
        return Err("Site URL is required".to_string());
    }
    let parsed = reqwest::Url::parse(raw).map_err(|_| format!("Invalid site URL: {}", site_url))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err("Site URL must start with http:// or https://".to_string());
    }
    Ok(raw.trim_end_matches('/').to_string())
}

fn parse_value_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    value
        .as_str()
        .and_then(|v| v.trim().parse::<i64>().ok())
}

fn parse_value_string(value: &Value) -> Option<String> {
    value.as_str().and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

fn parse_snapshot(value: &Value) -> LoginExtractResult {
    LoginExtractResult {
        user_id: value.get("user_id").and_then(parse_value_i64),
        username: value.get("username").and_then(parse_value_string),
        system_name: value.get("system_name").and_then(parse_value_string),
        access_token: value.get("access_token").and_then(parse_value_string),
        supports_checkin: value.get("supports_checkin").and_then(|v| v.as_bool()),
        can_checkin: value.get("can_checkin").and_then(|v| v.as_bool()),
    }
}

pub fn pick_free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("Failed to allocate free port: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to read allocated port: {}", e))?
        .port();
    Ok(port)
}

pub fn resolve_profile_dir(data_dir: &Path, profile_mode: &str) -> Result<PathBuf, String> {
    let root = data_dir.join("browser-profiles");
    fs::create_dir_all(&root)
        .map_err(|e| format!("Failed to create profile root directory: {}", e))?;

    let mode = normalize_profile_mode(profile_mode);
    let profile_dir = if mode == "isolated" {
        let slot = next_isolated_slot(&root)?;
        root.join(format!("slot-{}", slot))
    } else {
        root.join("main")
    };

    fs::create_dir_all(profile_dir.join("Default"))
        .map_err(|e| format!("Failed to create profile directory: {}", e))?;
    Ok(profile_dir)
}

fn next_isolated_slot(root: &Path) -> Result<u32, String> {
    let mut max_slot: u32 = 1;
    let entries = fs::read_dir(root)
        .map_err(|e| format!("Failed to read profile root directory: {}", e))?;

    for entry in entries {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(slot_raw) = name.strip_prefix("slot-") {
            if let Ok(slot) = slot_raw.parse::<u32>() {
                if slot > max_slot {
                    max_slot = slot;
                }
            }
        }
    }

    Ok(max_slot + 1)
}

pub fn detect_chrome_executable() -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(path) = std::env::var("CHROME_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("GOOGLE_CHROME_BIN") {
        candidates.push(PathBuf::from(path));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local)
                    .join("Google")
                    .join("Chrome")
                    .join("Application")
                    .join("chrome.exe"),
            );
            candidates.push(
                PathBuf::from(&local)
                    .join("Microsoft")
                    .join("Edge")
                    .join("Application")
                    .join("msedge.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                PathBuf::from(&program_files)
                    .join("Google")
                    .join("Chrome")
                    .join("Application")
                    .join("chrome.exe"),
            );
            candidates.push(
                PathBuf::from(&program_files)
                    .join("Microsoft")
                    .join("Edge")
                    .join("Application")
                    .join("msedge.exe"),
            );
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            candidates.push(
                PathBuf::from(&program_files_x86)
                    .join("Google")
                    .join("Chrome")
                    .join("Application")
                    .join("chrome.exe"),
            );
            candidates.push(
                PathBuf::from(&program_files_x86)
                    .join("Microsoft")
                    .join("Edge")
                    .join("Application")
                    .join("msedge.exe"),
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        candidates.push(
            PathBuf::from("/Applications")
                .join("Google Chrome.app")
                .join("Contents")
                .join("MacOS")
                .join("Google Chrome"),
        );
        candidates.push(
            PathBuf::from("/Applications")
                .join("Microsoft Edge.app")
                .join("Contents")
                .join("MacOS")
                .join("Microsoft Edge"),
        );
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/google-chrome"));
        candidates.push(PathBuf::from("/usr/bin/google-chrome-stable"));
        candidates.push(PathBuf::from("/usr/bin/chromium"));
        candidates.push(PathBuf::from("/usr/bin/chromium-browser"));
        candidates.push(PathBuf::from("/usr/bin/microsoft-edge"));
    }

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err("Chrome/Edge executable not found. Please install Chrome or set CHROME_PATH.".to_string())
}

pub async fn open_browser_for_login(
    site_url: &str,
    profile_dir: &Path,
    debug_port: u16,
) -> Result<PathBuf, String> {
    let normalized_url = normalize_site_url(site_url)?;
    let chrome_path = detect_chrome_executable()?;

    let args = [
        format!("--remote-debugging-port={}", debug_port),
        "--remote-allow-origins=*".to_string(),
        format!("--user-data-dir={}", profile_dir.to_string_lossy()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-session-crashed-bubble".to_string(),
        "--disable-features=Translate,AutomationControlled".to_string(),
        normalized_url,
    ];

    Command::new(&chrome_path)
        .args(args)
        .spawn()
        .map_err(|e| format!("Failed to launch browser: {}", e))?;

    wait_debugger_ready(debug_port).await?;
    Ok(chrome_path)
}

async fn wait_debugger_ready(debug_port: u16) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!("http://127.0.0.1:{}/json/version", debug_port);
    for _ in 0..80 {
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    Err(format!(
        "Browser debug port {} did not become ready. Make sure Chrome opened successfully.",
        debug_port
    ))
}

async fn list_debug_targets(debug_port: u16) -> Result<Vec<CdpTarget>, String> {
    let url = format!("http://127.0.0.1:{}/json/list", debug_port);
    reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to query browser debug targets: {}", e))?
        .json::<Vec<CdpTarget>>()
        .await
        .map_err(|e| format!("Failed to parse browser debug targets: {}", e))
}

fn pick_target_ws_url(targets: &[CdpTarget], site_url: &str) -> Option<String> {
    let host = reqwest::Url::parse(site_url)
        .ok()
        .and_then(|v| v.host_str().map(|s| s.to_string()));

    if let Some(hostname) = host {
        for target in targets {
            if target.target_type != "page" {
                continue;
            }
            let target_host = reqwest::Url::parse(&target.url)
                .ok()
                .and_then(|v| v.host_str().map(|s| s.to_string()));
            if target_host.as_deref() == Some(hostname.as_str()) {
                if let Some(ws_url) = target.ws_url.clone() {
                    return Some(ws_url);
                }
            }
        }
    }

    targets
        .iter()
        .find(|target| target.target_type == "page" && target.ws_url.is_some())
        .and_then(|target| target.ws_url.clone())
}

struct CdpClient {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
}

impl CdpClient {
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("Failed to connect to browser debug websocket: {}", e))?;
        Ok(Self { ws, next_id: 1 })
    }

    async fn call_method(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let payload = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        self.ws
            .send(Message::Text(payload.to_string()))
            .await
            .map_err(|e| format!("Failed to send CDP request: {}", e))?;

        while let Some(message) = self.ws.next().await {
            let message = message.map_err(|e| format!("Failed to read CDP response: {}", e))?;
            match message {
                Message::Text(text) => {
                    let response: Value =
                        serde_json::from_str(&text).map_err(|e| format!("Invalid CDP JSON: {}", e))?;
                    if response.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        if let Some(error) = response.get("error") {
                            let msg = error
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown CDP error");
                            return Err(format!("CDP call {} failed: {}", method, msg));
                        }
                        return Ok(response
                            .get("result")
                            .cloned()
                            .unwrap_or(Value::Null));
                    }
                }
                Message::Binary(binary) => {
                    if let Ok(text) = String::from_utf8(binary.to_vec()) {
                        let response: Value = serde_json::from_str(&text)
                            .map_err(|e| format!("Invalid binary CDP JSON: {}", e))?;
                        if response.get("id").and_then(|v| v.as_u64()) == Some(id) {
                            if let Some(error) = response.get("error") {
                                let msg = error
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown CDP error");
                                return Err(format!("CDP call {} failed: {}", method, msg));
                            }
                            return Ok(response
                                .get("result")
                                .cloned()
                                .unwrap_or(Value::Null));
                        }
                    }
                }
                Message::Ping(data) => {
                    let _ = self.ws.send(Message::Pong(data)).await;
                }
                Message::Close(_) => {
                    return Err("CDP websocket closed unexpectedly".to_string());
                }
                _ => {}
            }
        }

        Err("CDP websocket ended without response".to_string())
    }

    async fn evaluate_json(&mut self, expression: &str) -> Result<Value, String> {
        let result = self
            .call_method(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        if let Some(exception) = result.get("exceptionDetails") {
            let message = exception
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("JavaScript execution failed");
            return Err(format!("Browser script failed: {}", message));
        }

        let object = result
            .get("result")
            .cloned()
            .unwrap_or(Value::Null);

        if let Some(value) = object.get("value") {
            return Ok(value.clone());
        }

        if let Some(unserializable) = object.get("unserializableValue").and_then(|v| v.as_str()) {
            return Ok(Value::String(unserializable.to_string()));
        }

        Ok(Value::Null)
    }
}

pub async fn extract_login_snapshot(
    site_url: &str,
    debug_port: u16,
) -> Result<LoginExtractResult, String> {
    let normalized_url = normalize_site_url(site_url)?;
    let targets = list_debug_targets(debug_port).await?;
    if targets.is_empty() {
        return Err("No browser pages found. Please keep the login page open.".to_string());
    }

    let ws_url = pick_target_ws_url(&targets, &normalized_url)
        .ok_or_else(|| "No suitable browser page found for this site. Please open the target site tab.".to_string())?;

    let mut cdp = CdpClient::connect(&ws_url).await?;
    let snapshot_raw = cdp.evaluate_json(LOCAL_STORAGE_SNAPSHOT_SCRIPT).await?;
    let mut snapshot = parse_snapshot(&snapshot_raw);

    if snapshot.user_id.is_none() {
        return Err("Login user not detected. Please complete login in browser and refresh the page.".to_string());
    }

    let token_missing = snapshot
        .access_token
        .as_deref()
        .map(|v| v.trim().is_empty())
        .unwrap_or(true);

    if token_missing {
        let script = build_create_token_script(snapshot.user_id.unwrap_or_default());
        let token_result = cdp.evaluate_json(&script).await?;
        let ok = token_result
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if ok {
            snapshot.access_token = token_result
                .get("token")
                .and_then(parse_value_string);
        } else {
            let message = token_result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Failed to create access token");
            return Err(format!(
                "Access token not found in localStorage and token creation failed: {}",
                message
            ));
        }
    }

    Ok(snapshot)
}
