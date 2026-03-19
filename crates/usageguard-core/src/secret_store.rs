use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(any(test, target_os = "windows"))]
use std::fs;
use std::path::PathBuf;

const APP_DIR_NAME: &str = "usage-guard";
const CONFIG_DIR_OVERRIDE_ENV: &str = "USAGEGUARD_CONFIG_DIR_OVERRIDE";
const SECRET_STORE_FILE_NAME: &str = "secrets.bin";
#[cfg(target_os = "linux")]
const SECRET_STORE_KEYRING_USER: &str = "secret-payload";
const SECRET_PAYLOAD_VERSION: u32 = 1;
const WINDOWS_BACKEND_ID: &str = "windows-dpapi";
#[cfg(target_os = "linux")]
const LINUX_BACKEND_ID: &str = "linux-secret-service";
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
const UNSUPPORTED_BACKEND_ID: &str = "unsupported";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretPayload {
    pub version: u32,
    #[serde(default)]
    pub provider_api_keys: HashMap<String, String>,
}

impl Default for SecretPayload {
    fn default() -> Self {
        Self {
            version: SECRET_PAYLOAD_VERSION,
            provider_api_keys: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LegacySecretPayload {
    #[serde(default)]
    provider_api_keys: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecureStorageStatus {
    pub available: bool,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl SecureStorageStatus {
    fn available(backend: &str) -> Self {
        Self {
            available: true,
            backend: backend.to_string(),
            detail: None,
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn unavailable(backend: &str, detail: impl Into<String>) -> Self {
        Self {
            available: false,
            backend: backend.to_string(),
            detail: Some(detail.into()),
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

    pub fn status() -> SecureStorageStatus {
        secure_storage_status()
    }

    pub fn load() -> Result<SecretPayload> {
        let Some(raw) = load_payload_bytes()? else {
            return Ok(SecretPayload::default());
        };

        let payload = serde_json::from_slice::<LegacySecretPayload>(&raw)
            .context("Secret store is invalid JSON")?;

        let payload = SecretPayload {
            version: SECRET_PAYLOAD_VERSION,
            provider_api_keys: payload.provider_api_keys,
        };

        if payload.version != SECRET_PAYLOAD_VERSION {
            return Err(anyhow!(
                "Unsupported secret store version {}",
                payload.version
            ));
        }

        Ok(payload)
    }

    pub fn load_or_default() -> SecretPayload {
        Self::load().unwrap_or_default()
    }

    pub fn save(payload: &SecretPayload) -> Result<()> {
        let mut normalized = payload.clone();
        normalized.version = SECRET_PAYLOAD_VERSION;
        let raw = serde_json::to_vec(&normalized)?;
        save_payload_bytes(&raw)
    }

    #[cfg(test)]
    pub fn clear() -> Result<()> {
        clear_payload_bytes()
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
fn secure_storage_status() -> SecureStorageStatus {
    SecureStorageStatus::available(WINDOWS_BACKEND_ID)
}

#[cfg(target_os = "linux")]
fn secure_storage_status() -> SecureStorageStatus {
    let entry = match linux_secret_entry() {
        Ok(entry) => entry,
        Err(error) => {
            return SecureStorageStatus::unavailable(
                LINUX_BACKEND_ID,
                format!("Could not initialize Linux secure storage: {error}"),
            )
        }
    };

    match entry.get_secret() {
        Ok(_) | Err(keyring::Error::NoEntry) => SecureStorageStatus::available(LINUX_BACKEND_ID),
        Err(error) => SecureStorageStatus::unavailable(
            LINUX_BACKEND_ID,
            format!("Could not access the Linux Secret Service keyring: {error}"),
        ),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn secure_storage_status() -> SecureStorageStatus {
    SecureStorageStatus::unavailable(
        UNSUPPORTED_BACKEND_ID,
        "Secure secret persistence is only implemented for Windows and Linux in this release",
    )
}

#[cfg(target_os = "windows")]
fn load_payload_bytes() -> Result<Option<Vec<u8>>> {
    let path = SecretStore::path()?;
    if !path.exists() {
        return Ok(None);
    }

    let encrypted = fs::read(&path)
        .with_context(|| format!("Unable to read secret store: {}", path.display()))?;
    let decrypted = decrypt_bytes(&encrypted)
        .with_context(|| format!("Unable to decrypt secret store: {}", path.display()))?;
    Ok(Some(decrypted))
}

#[cfg(target_os = "linux")]
fn load_payload_bytes() -> Result<Option<Vec<u8>>> {
    let entry = linux_secret_entry()?;
    match entry.get_secret() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(anyhow!(
            "Unable to read secure secret store from the Linux keyring: {error}"
        )),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn load_payload_bytes() -> Result<Option<Vec<u8>>> {
    Err(anyhow!(
        "Secure secret persistence is only implemented for Windows and Linux in this release"
    ))
}

#[cfg(target_os = "windows")]
fn save_payload_bytes(raw: &[u8]) -> Result<()> {
    let path = SecretStore::path()?;
    let dir = path
        .parent()
        .context("Secret store parent directory missing")?;
    fs::create_dir_all(dir)
        .with_context(|| format!("Unable to create secret store dir: {}", dir.display()))?;

    let encrypted = encrypt_bytes(raw)?;
    fs::write(&path, encrypted)
        .with_context(|| format!("Unable to write secret store: {}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn save_payload_bytes(raw: &[u8]) -> Result<()> {
    linux_secret_entry()?
        .set_secret(raw)
        .map_err(|error| anyhow!("Unable to write secret store to the Linux keyring: {error}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn save_payload_bytes(_raw: &[u8]) -> Result<()> {
    Err(anyhow!(
        "Secure secret persistence is only implemented for Windows and Linux in this release"
    ))
}

#[cfg(all(test, target_os = "windows"))]
fn clear_payload_bytes() -> Result<()> {
    let path = SecretStore::path()?;
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("Unable to remove secret store: {}", path.display()))?;
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
fn clear_payload_bytes() -> Result<()> {
    let entry = linux_secret_entry()?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(anyhow!(
            "Unable to clear secret store from the Linux keyring: {error}"
        )),
    }
}

#[cfg(all(test, not(any(target_os = "windows", target_os = "linux"))))]
fn clear_payload_bytes() -> Result<()> {
    Err(anyhow!(
        "Secure secret persistence is only implemented for Windows and Linux in this release"
    ))
}

#[cfg(target_os = "linux")]
fn linux_secret_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(APP_DIR_NAME, SECRET_STORE_KEYRING_USER)
        .map_err(|error| anyhow!("Unable to create Linux keyring entry: {error}"))
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

        let raw = serde_json::to_vec(&payload).unwrap();
        let encrypted = encrypt_bytes(&raw).unwrap();
        let decrypted = decrypt_bytes(&encrypted).unwrap();
        let loaded = serde_json::from_slice::<SecretPayload>(&decrypted).unwrap();
        assert_eq!(loaded, payload);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn legacy_secret_payload_loads_and_drops_oauth_fields() {
        with_test_dir("legacy", |app_dir| {
            fs::create_dir_all(app_dir).unwrap();
            let legacy = serde_json::json!({
                "version": 1,
                "provider_api_keys": {
                    "openai": "sk-test"
                },
                "openai_oauth": {
                    "refresh_token": "refresh-token",
                    "account_id": "acct_123",
                    "plan_type": "plus"
                },
                "anthropic_oauth": {
                    "refresh_token": "claude-refresh",
                    "subscription_type": "max",
                    "rate_limit_tier": "premium"
                }
            });
            let encrypted =
                encrypt_bytes(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
            fs::write(app_dir.join(SECRET_STORE_FILE_NAME), encrypted).unwrap();

            let loaded = SecretStore::load().unwrap();
            assert_eq!(
                loaded.provider_api_keys.get("openai"),
                Some(&"sk-test".to_string())
            );
            assert_eq!(
                loaded,
                SecretPayload {
                    version: SECRET_PAYLOAD_VERSION,
                    provider_api_keys: HashMap::from([("openai".into(), "sk-test".into())]),
                }
            );
        });
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
