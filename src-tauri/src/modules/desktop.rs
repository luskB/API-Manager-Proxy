use std::fs;
use std::path::{Path, PathBuf};

const STARTUP_SCRIPT_NAME: &str = "APIManager.cmd";

pub fn sync_launch_on_startup(enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let script_path = startup_script_path()?;
        if enabled {
            write_startup_script(&script_path)?;
        } else if script_path.exists() {
            fs::remove_file(&script_path)
                .map_err(|e| format!("Failed to remove startup script: {}", e))?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = enabled;

    Ok(())
}

#[cfg(target_os = "windows")]
fn startup_script_path() -> Result<PathBuf, String> {
    let base = dirs::config_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "Cannot determine startup directory".to_string())?;
    let startup_dir = base
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup");
    if !startup_dir.exists() {
        fs::create_dir_all(&startup_dir)
            .map_err(|e| format!("Failed to create startup directory: {}", e))?;
    }
    Ok(startup_dir.join(STARTUP_SCRIPT_NAME))
}

#[cfg(target_os = "windows")]
fn write_startup_script(path: &Path) -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Failed to locate current executable: {}", e))?;
    let escaped = current_exe.to_string_lossy().replace('"', "\"\"");
    let script = format!("@echo off\r\nstart \"\" \"{}\"\r\n", escaped);
    fs::write(path, script).map_err(|e| format!("Failed to write startup script: {}", e))
}
