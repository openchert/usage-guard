use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const APP_DIR_NAME: &str = "usage-guard";
const CONFIG_DIR_OVERRIDE_ENV: &str = "USAGEGUARD_CONFIG_DIR_OVERRIDE";
const SECRET_STORE_FILE_NAME: &str = "secrets.bin";
const SECRET_PAYLOAD_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct OpenAiOAuthSecret {
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub plan_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AnthropicOAuthSecret {
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub subscription_type: String,
    #[serde(default)]
    pub rate_limit_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretPayload {
    pub version: u32,
    #[serde(default)]
    pub provider_api_keys: HashMap<String, String>,
    #[serde(default)]
    pub openai_oauth: OpenAiOAuthSecret,
    #[serde(default)]
    pub anthropic_oauth: AnthropicOAuthSecret,
}

impl Default for SecretPayload {
    fn default() -> Self {
        Self {
            version: SECRET_PAYLOAD_VERSION,
            provider_api_keys: HashMap::new(),
            openai_oauth: OpenAiOAuthSecret::default(),
            anthropic_oauth: AnthropicOAuthSecret::default(),
        }
    }
}

pub struct SecretStore;

#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

impl SecretStore {
    pub fn path() -> Result<PathBuf> {
        Ok(app_config_dir()?.join(SECRET_STORE_FILE_NAME))
    }

    pub fn load() -> Result<SecretPayload> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(SecretPayload::default());
        }

        let encrypted = fs::read(&path)
            .with_context(|| format!("Unable to read secret store: {}", path.display()))?;
        let decrypted = decrypt_bytes(&encrypted)
            .with_context(|| format!("Unable to decrypt secret store: {}", path.display()))?;
        let payload = serde_json::from_slice::<SecretPayload>(&decrypted)
            .with_context(|| format!("Secret store is invalid JSON: {}", path.display()))?;

        if payload.version != SECRET_PAYLOAD_VERSION {
            return Err(anyhow!(
                "Unsupported secret store version {} in {}",
                payload.version,
                path.display()
            ));
        }

        Ok(payload)
    }

    pub fn load_or_default() -> SecretPayload {
        Self::load().unwrap_or_default()
    }

    pub fn save(payload: &SecretPayload) -> Result<()> {
        let path = Self::path()?;
        let dir = path
            .parent()
            .context("Secret store parent directory missing")?;
        fs::create_dir_all(dir)
            .with_context(|| format!("Unable to create secret store dir: {}", dir.display()))?;

        let mut normalized = payload.clone();
        normalized.version = SECRET_PAYLOAD_VERSION;
        let raw = serde_json::to_vec(&normalized)?;
        let encrypted = encrypt_bytes(&raw)?;
        fs::write(&path, encrypted)
            .with_context(|| format!("Unable to write secret store: {}", path.display()))?;
        Ok(())
    }

    pub fn clear() -> Result<()> {
        let path = Self::path()?;
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Unable to remove secret store: {}", path.display()))?;
        }
        Ok(())
    }
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

#[cfg(target_os = "windows")]
fn encrypt_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut input = CRYPT_INTEGER_BLOB {
        cbData: raw.len() as u32,
        pbData: raw.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    let ok = unsafe {
        CryptProtectData(
            &mut input,
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };

    if ok == 0 {
        return Err(anyhow!(std::io::Error::last_os_error()));
    }

    let encrypted = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let bytes = slice.to_vec();
        let _ = LocalFree(output.pbData.cast());
        bytes
    };

    Ok(encrypted)
}

#[cfg(target_os = "windows")]
fn decrypt_bytes(encrypted: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let mut input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };

    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };

    if ok == 0 {
        return Err(anyhow!(std::io::Error::last_os_error()));
    }

    let decrypted = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let bytes = slice.to_vec();
        let _ = LocalFree(output.pbData.cast());
        bytes
    };

    Ok(decrypted)
}

#[cfg(not(target_os = "windows"))]
fn encrypt_bytes(_raw: &[u8]) -> Result<Vec<u8>> {
    Err(anyhow!(
        "Secure secret persistence is only implemented for Windows in this release"
    ))
}

#[cfg(not(target_os = "windows"))]
fn decrypt_bytes(_encrypted: &[u8]) -> Result<Vec<u8>> {
    Err(anyhow!(
        "Secure secret persistence is only implemented for Windows in this release"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn with_test_dir(name: &str, test: impl FnOnce(&Path)) {
        let _guard = test_env_lock().lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "usageguard_secret_store_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, &root);
        test(&root.join(APP_DIR_NAME));
        std::env::remove_var(CONFIG_DIR_OVERRIDE_ENV);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dpapi_round_trip() {
        let mut payload = SecretPayload::default();
        payload
            .provider_api_keys
            .insert("openai".into(), "sk-test".into());
        payload.openai_oauth.refresh_token = "refresh-token".into();
        payload.openai_oauth.account_id = "acct_123".into();
        payload.openai_oauth.plan_type = "plus".into();
        payload.anthropic_oauth.refresh_token = "claude-refresh".into();
        payload.anthropic_oauth.subscription_type = "max".into();
        payload.anthropic_oauth.rate_limit_tier = "premium".into();

        let raw = serde_json::to_vec(&payload).unwrap();
        let encrypted = encrypt_bytes(&raw).unwrap();
        let decrypted = decrypt_bytes(&encrypted).unwrap();
        let loaded = serde_json::from_slice::<SecretPayload>(&decrypted).unwrap();
        assert_eq!(loaded, payload);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn corrupted_secret_store_fails_closed() {
        with_test_dir("corrupt", |app_dir| {
            fs::create_dir_all(app_dir).unwrap();
            fs::write(app_dir.join(SECRET_STORE_FILE_NAME), b"not-dpapi").unwrap();
            assert!(SecretStore::load().is_err());
            assert_eq!(SecretStore::load_or_default(), SecretPayload::default());
        });
    }
}
