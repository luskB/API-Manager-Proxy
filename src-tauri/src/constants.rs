/// Default proxy listen port
pub const DEFAULT_PORT: u16 = 8045;

/// Default upstream request timeout in seconds
pub const DEFAULT_REQUEST_TIMEOUT: u64 = 120;

/// Public app name shown in desktop UI and bundle metadata.
pub const APP_NAME: &str = "APIManagerProxy";

/// Public app identifier used by desktop integrations.
pub const APP_IDENTIFIER: &str = "com.luskb.apimanagerproxy";

/// Application data directory name.
pub const DATA_DIR_NAME: &str = APP_NAME;

/// Legacy application data directory name kept for migration.
pub const LEGACY_DATA_DIR_NAME: &str = "APIManager";

/// Configuration file name.
pub const CONFIG_FILE_NAME: &str = "apimanagerproxy_config.json";

/// Legacy configuration file name kept for migration.
pub const LEGACY_CONFIG_FILE_NAME: &str = "apimanager_config.json";

/// CLI backup suffix created when syncing external tool configs.
pub const CLI_BACKUP_SUFFIX: &str = ".apimanagerproxy.bak";

/// Legacy CLI backup suffix kept for restore compatibility.
pub const LEGACY_CLI_BACKUP_SUFFIX: &str = ".apimanager.bak";

/// Application version from Cargo.toml
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
