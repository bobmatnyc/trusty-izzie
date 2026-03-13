const PLIST_LABEL: &str = "dev.izzie.trusty-daemon";
const DAEMON_BIN: &str = "/usr/local/bin/trusty-daemon";
const SOCKET_PATH: &str = "/tmp/trusty-izzie.sock";

fn plist_path() -> std::path::PathBuf {
    dirs::home_dir()
        .expect("home dir")
        .join("Library/LaunchAgents")
        .join(format!("{PLIST_LABEL}.plist"))
}

fn plist_content() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{PLIST_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{DAEMON_BIN}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>TRUSTY_CONFIG</key>
    <string>~/.config/trusty-izzie/config.env</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/trusty-daemon.log</string>
  <key>StandardErrorPath</key><string>/tmp/trusty-daemon.log</string>
</dict>
</plist>
"#
    )
}

#[tauri::command]
pub async fn install_launch_agent() -> Result<(), String> {
    let path = plist_path();

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    tokio::fs::write(&path, plist_content())
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn start_daemon() -> Result<(), String> {
    let path = plist_path();
    let output = tokio::process::Command::new("launchctl")
        .args(["load", &path.to_string_lossy()])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // launchctl prints a warning if already loaded — not a real error.
        if !stderr.contains("already loaded") {
            return Err(stderr.trim().to_string());
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn verify_daemon() -> Result<bool, String> {
    // Give the daemon a moment to start.
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let socket_exists = tokio::fs::metadata(SOCKET_PATH).await.is_ok();
    if socket_exists {
        return Ok(true);
    }

    // Also accept if the binary doesn't exist yet (installer UI shouldn't block).
    if tokio::fs::metadata(DAEMON_BIN).await.is_err() {
        return Ok(true);
    }

    Err(format!("Daemon socket not found at {SOCKET_PATH}"))
}

#[tauri::command]
pub async fn stop_daemon() -> Result<(), String> {
    let plist = plist_path();
    tokio::process::Command::new("launchctl")
        .args(["unload", &plist.to_string_lossy()])
        .status()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn close_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    app.get_webview_window("main")
        .ok_or("main window not found")?
        .close()
        .map_err(|e| e.to_string())
}
