use anyhow::{Context, Result};
use std::path::PathBuf;

const APP_DIR_NAME: &str = "usage-guard";
const CONFIG_DIR_OVERRIDE_ENV: &str = "USAGEGUARD_CONFIG_DIR_OVERRIDE";

#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub fn app_config_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var(CONFIG_DIR_OVERRIDE_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join(APP_DIR_NAME));
        }
    }

    let base = dirs::config_dir().context("Unable to resolve config directory")?;
    Ok(base.join(APP_DIR_NAME))
}
