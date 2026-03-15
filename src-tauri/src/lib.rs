pub mod commands;
pub mod constants;
pub mod error;
pub mod models;
pub mod modules;
pub mod proxy;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use commands::ProxyServiceState;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

use crate::constants::APP_NAME;

#[derive(Default)]
struct AppLifecycleState {
    allow_exit: Arc<AtomicBool>,
}

impl AppLifecycleState {
    fn request_exit(&self) {
        self.allow_exit.store(true, Ordering::SeqCst);
    }

    fn consume_exit_request(&self) -> bool {
        self.allow_exit.swap(false, Ordering::SeqCst)
    }
}

pub fn run() {
    modules::logger::init_logger();

    let is_headless = std::env::args().any(|arg| arg == "--headless");
    if is_headless {
        run_headless();
        return;
    }

    let _single_instance_guard = match modules::single_instance::acquire_single_instance_guard() {
        Ok(Some(guard)) => guard,
        Ok(None) => return,
        Err(error) => {
            tracing::error!("Failed to initialize single-instance guard: {}", error);
            return;
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(ProxyServiceState::new())
        .manage(AppLifecycleState::default())
        .setup(|app| {
            setup_tray(app)?;

            let state = app.state::<ProxyServiceState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let config = modules::config::load_app_config();
                if config.proxy.api_key.is_empty() && config.proxy_accounts.is_empty() {
                    tracing::info!("No proxy config found, skipping auto-start");
                    return;
                }
                tracing::info!("Auto-starting proxy server on port {}", config.proxy.port);
                match proxy::server::start_server(&config.proxy, &config.proxy_accounts).await {
                    Ok(axum_server) => {
                        let monitor_ref = axum_server.monitor.clone();
                        let token_manager_ref = axum_server.token_manager.clone();
                        let security_ref = axum_server.security.clone();
                        let model_router_ref = axum_server.model_router.clone();
                        {
                            let mut server = state.server.lock().await;
                            server.set_server(axum_server);
                        }
                        {
                            let mut monitor = state.monitor.write().await;
                            *monitor = Some(monitor_ref);
                        }
                        {
                            let mut tm = state.token_manager.write().await;
                            *tm = Some(token_manager_ref);
                        }
                        {
                            let mut security = state.security.write().await;
                            *security = Some(security_ref);
                        }
                        {
                            let mut model_router = state.model_router.write().await;
                            *model_router = Some(model_router_ref);
                        }
                        tracing::info!("Proxy server auto-started successfully");
                    }
                    Err(e) => {
                        tracing::error!("Failed to auto-start proxy server: {}", e);
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.app_handle().state::<AppLifecycleState>();
                if lifecycle.consume_exit_request() {
                    return;
                }

                if should_hide_to_tray() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::import_backup,
            commands::import_backup_from_text,
            commands::detect_browser_extension,
            commands::sync_from_browser,
            commands::load_config,
            commands::refresh_api_keys,
            commands::open_browser_login,
            commands::import_account_from_browser_login,
            commands::save_config,
            commands::proxy_start,
            commands::proxy_stop,
            commands::get_proxy_status,
            commands::get_logs,
            commands::replay_request,
            commands::get_available_models,
            commands::get_proxy_stats,
            commands::get_proxy_stats_view,
            commands::get_token_stats_view,
            commands::get_proxy_model_catalog,
            commands::get_proxy_model_prices,
            commands::validate_api_key,
            commands::list_hub_accounts,
            commands::refresh_hub_balances,
            commands::refresh_selected_hub_balances,
            commands::detect_hub_account,
            commands::detect_all_hub_accounts,
            commands::hub_checkin_account,
            commands::hub_fetch_api_tokens,
            commands::hub_create_api_token,
            commands::hub_delete_api_token,
            commands::hub_fetch_user_groups,
            commands::hub_fetch_model_pricing,
            proxy::cli_sync::get_cli_sync_status,
            proxy::cli_sync::execute_cli_sync,
            proxy::cli_sync::execute_cli_restore,
            proxy::cli_sync::get_cli_config_content,
            proxy::cli_sync::generate_cli_config,
            proxy::cli_sync::write_cli_config,
            proxy::cli_sync::probe_cli_compatibility,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", format!("Show {APP_NAME}")).build(app)?;
    let quit = MenuItemBuilder::with_id("quit", format!("Quit {APP_NAME}")).build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

    let mut builder = TrayIconBuilder::with_id("main-tray").menu(&menu);
    if let Some(icon) = tray_icon_image(app) {
        builder = builder.icon(icon);
    }

    builder
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => quit_application(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn tray_icon_image(app: &tauri::App) -> Option<Image<'static>> {
    if let Some(icon) = app.default_window_icon() {
        return Some(icon.clone().to_owned());
    }

    Image::from_bytes(include_bytes!("../icons/icon.png")).ok()
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn quit_application(app: &tauri::AppHandle) {
    let lifecycle = app.state::<AppLifecycleState>();
    lifecycle.request_exit();
    app.exit(0);
}

fn should_hide_to_tray() -> bool {
    matches!(
        modules::config::load_app_config().desktop.close_behavior,
        models::CloseBehavior::Tray
    )
}

fn run_headless() {
    tracing::info!("Starting {} in headless mode...", APP_NAME);

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let mut config = modules::config::load_app_config();

        if let Ok(key) = std::env::var("ABV_API_KEY").or_else(|_| std::env::var("API_KEY")) {
            config.proxy.api_key = key;
        }
        if let Ok(pw) = std::env::var("ABV_WEB_PASSWORD").or_else(|_| std::env::var("WEB_PASSWORD"))
        {
            config.proxy.admin_password = Some(pw);
        }
        if let Ok(port) = std::env::var("PORT")
            .or_else(|_| std::env::var("ABV_PORT"))
            .and_then(|p| p.parse::<u16>().map_err(|_| std::env::VarError::NotPresent))
        {
            config.proxy.port = port;
        }
        if let Ok(mode) = std::env::var("ABV_AUTH_MODE").or_else(|_| std::env::var("AUTH_MODE")) {
            config.proxy.auth_mode = match mode.to_lowercase().as_str() {
                "off" => models::ProxyAuthMode::Off,
                "strict" => models::ProxyAuthMode::Strict,
                "all_except_health" => models::ProxyAuthMode::AllExceptHealth,
                _ => models::ProxyAuthMode::Auto,
            };
        }

        if let Ok(val) = std::env::var("ABV_BIND_LOCAL_ONLY") {
            if val == "true" || val == "1" {
                config.proxy.allow_lan_access = false;
            }
        } else {
            config.proxy.allow_lan_access = true;
        }

        tracing::info!(
            port = config.proxy.port,
            api_key = %mask_key(&config.proxy.api_key),
            auth_mode = ?config.proxy.auth_mode,
            lan_access = config.proxy.allow_lan_access,
            accounts = config.proxy_accounts.len(),
            "Headless configuration loaded"
        );

        match proxy::server::start_server(&config.proxy, &config.proxy_accounts).await {
            Ok(mut server) => {
                tracing::info!(
                    "Proxy server running on {}:{}",
                    if config.proxy.allow_lan_access { "0.0.0.0" } else { "127.0.0.1" },
                    config.proxy.port
                );

                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen for ctrl-c");
                tracing::info!("Received Ctrl-C, shutting down...");
                server.stop().await;
            }
            Err(e) => {
                tracing::error!("Failed to start proxy server: {}", e);
                std::process::exit(1);
            }
        }
    });
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        "***".to_string()
    } else {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    }
}
