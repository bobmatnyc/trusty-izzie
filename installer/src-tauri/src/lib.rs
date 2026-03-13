mod commands;
mod installer;

use commands::{config, daemon, oauth};

#[tauri::command]
async fn check_rust_installed() -> Result<String, String> {
    installer::check_rust().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_install_status() -> Result<installer::InstallStatus, String> {
    installer::get_status().await.map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            check_rust_installed,
            get_install_status,
            config::write_config,
            oauth::start_google_oauth,
            oauth::poll_oauth_result,
            daemon::install_launch_agent,
            daemon::start_daemon,
            daemon::verify_daemon,
            daemon::close_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
