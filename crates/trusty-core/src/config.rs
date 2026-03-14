//! Config loading from file + environment variables.

use trusty_models::config::AppConfig;

use crate::error::{Result, TrustyError};

/// Load application configuration.
///
/// Resolution order (later values override earlier ones):
/// 1. `config/default.toml` (shipped with the binary)
/// 2. The file at `config_path`, if provided
/// 3. Environment variables prefixed with `TRUSTY__`
///
/// After loading, dev-mode defaults are applied when `instance.env == "dev"`
/// unless the caller already set the relevant `TRUSTY__*` env vars explicitly.
///
/// # Errors
/// Returns [`TrustyError::Config`] if the configuration cannot be parsed.
pub async fn load_config(config_path: Option<&str>) -> Result<AppConfig> {
    use config::{Config, Environment, File};

    let mut builder =
        Config::builder().add_source(File::with_name("config/default").required(false));

    if let Some(path) = config_path {
        builder = builder.add_source(File::with_name(path).required(true));
    }

    builder = builder.add_source(
        Environment::with_prefix("TRUSTY")
            .separator("__")
            .try_parsing(true),
    );

    let cfg = builder
        .build()
        .map_err(|e| TrustyError::Config(e.to_string()))?;

    let mut app: AppConfig = cfg
        .try_deserialize()
        .map_err(|e| TrustyError::Config(e.to_string()))?;

    // Apply dev-mode defaults when instance.env == "dev" and the caller has not
    // explicitly set the corresponding TRUSTY__* env vars.
    if app.instance.env == "dev" {
        if std::env::var("TRUSTY__STORAGE__DATA_DIR").is_err() {
            app.storage.data_dir = "~/.local/share/trusty-izzie-dev".to_string();
        }
        if std::env::var("TRUSTY__API__PORT").is_err() {
            app.api.port = 3458;
        }
        if std::env::var("TRUSTY__DAEMON__IPC_SOCKET").is_err() {
            app.daemon.ipc_socket = "/tmp/trusty-izzie-dev.sock".to_string();
        }
        if app.instance.label.is_empty() {
            app.instance.label = "DEV".to_string();
        }
    }

    Ok(app)
}
