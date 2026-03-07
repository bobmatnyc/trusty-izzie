use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct InstallStatus {
    pub rust_installed: bool,
    pub rust_version: Option<String>,
    pub izzie_installed: bool,
    pub data_dir_exists: bool,
}

pub async fn check_rust() -> anyhow::Result<String> {
    let output = tokio::process::Command::new("rustc")
        .arg("--version")
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn get_status() -> anyhow::Result<InstallStatus> {
    let rust_output = tokio::process::Command::new("rustc")
        .arg("--version")
        .output()
        .await;

    let (rust_installed, rust_version) = match rust_output {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, Some(v))
        }
        _ => (false, None),
    };

    let data_dir = dirs::home_dir()
        .map(|h| h.join(".local/share/trusty-izzie"))
        .map(|p| p.exists())
        .unwrap_or(false);

    Ok(InstallStatus {
        rust_installed,
        rust_version,
        izzie_installed: data_dir,
        data_dir_exists: data_dir,
    })
}
