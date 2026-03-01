//! Config loading from file + environment variables.

use trusty_models::config::AppConfig;

use crate::error::{Result, TrustyError};

/// Load application configuration.
///
/// Resolution order (later values override earlier ones):
/// 1. `config/default.toml` (shipped with the binary)
/// 2. The file at `config_path`, if provided
/// 3. Environment variables prefixed with `TRUSTY_`
///
/// # Errors
/// Returns [`TrustyError::Config`] if the configuration cannot be parsed.
pub async fn load_config(config_path: Option<&str>) -> Result<AppConfig> {
    use config::{Config, Environment, File};

    let mut builder = Config::builder()
        .add_source(File::with_name("config/default").required(false));

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

    cfg.try_deserialize::<AppConfig>()
        .map_err(|e| TrustyError::Config(e.to_string()))
}
