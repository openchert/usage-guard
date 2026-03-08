mod secret_store;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use secret_store::{app_config_dir, OpenAiOAuthSecret, SecretPayload, SecretStore};

const KEYRING_SERVICE: &str = "usage-guard";
const ACCESS_TOKEN_EXPIRY_SKEW_SECS: i64 = 60;
const DEFAULT_ACCESS_TOKEN_LIFETIME_SECS: i64 = 45 * 60;

#[derive(Debug, Clone, Default)]
struct OpenAiSessionState {
    access_token: Option<String>,
    refresh_token: Option<String>,
    account_id: String,
    plan_type: String,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider: String,
    pub account_label: String,
    pub spent_usd: f64,
    pub limit_usd: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub inactive_hours: u32,
    pub source: String,
    #[serde(default)]
    pub status_code: Option<String>,
    #[serde(default)]
    pub status_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub level: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuietHours {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            enabled: true,
            start_hour: 23,
            end_hour: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiCredentials {
    pub openai_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
    pub mistral_api_key: Option<String>,
    pub groq_api_key: Option<String>,
    pub copilot_api_key: Option<String>,
    pub cursor_api_key: Option<String>,

    pub openai_costs_endpoint: Option<String>,
    pub anthropic_costs_endpoint: Option<String>,
    pub gemini_costs_endpoint: Option<String>,
    pub mistral_costs_endpoint: Option<String>,
    pub groq_costs_endpoint: Option<String>,
    pub copilot_costs_endpoint: Option<String>,
    pub cursor_costs_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: String,
    pub label: String,
    pub endpoint: String,
    pub auth_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAccount {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCatalogEntry {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub near_limit_ratio: f64,
    pub inactive_threshold_hours: u32,
    pub quiet_hours: QuietHours,
    pub api: ApiCredentials,
    #[serde(default)]
    pub provider_accounts: Vec<ProviderAccount>,
    #[serde(default)]
    pub profiles: Vec<ProviderProfile>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            near_limit_ratio: 0.85,
            inactive_threshold_hours: 8,
            quiet_hours: QuietHours::default(),
            api: ApiCredentials::default(),
            provider_accounts: vec![],
            profiles: vec![],
        }
    }
}

#[derive(Clone, Copy)]
enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Copy)]
enum AuthMode {
    Bearer,
    Raw,
    Basic,
}

#[derive(Clone)]
struct ProviderTemplate {
    id: &'static str,
    label: &'static str,
    env_prefix: &'static str,
    default_endpoint: Option<&'static str>,
    method: HttpMethod,
    auth_header: &'static str,
    auth_mode: AuthMode,
    extra_headers: Vec<(&'static str, &'static str)>,
    request_body: Option<Value>,
    usage_log_env: Option<&'static str>,
}

fn builtin_provider_templates() -> Vec<ProviderTemplate> {
    vec![
        ProviderTemplate {
            id: "openai",
            label: "OpenAI",
            env_prefix: "OPENAI",
            default_endpoint: Some("https://api.openai.com/v1/organization/costs"),
            method: HttpMethod::Get,
            auth_header: "Authorization",
            auth_mode: AuthMode::Bearer,
            extra_headers: vec![],
            request_body: None,
            usage_log_env: Some("OPENAI_USAGE_LOG"),
        },
        ProviderTemplate {
            id: "anthropic",
            label: "Anthropic",
            env_prefix: "ANTHROPIC",
            default_endpoint: Some("https://api.anthropic.com/v1/organizations/usage"),
            method: HttpMethod::Get,
            auth_header: "x-api-key",
            auth_mode: AuthMode::Raw,
            extra_headers: vec![("anthropic-version", "2023-06-01")],
            request_body: None,
            usage_log_env: Some("ANTHROPIC_USAGE_LOG"),
        },
        ProviderTemplate {
            id: "gemini",
            label: "Gemini",
            env_prefix: "GEMINI",
            default_endpoint: None,
            method: HttpMethod::Get,
            auth_header: "Authorization",
            auth_mode: AuthMode::Bearer,
            extra_headers: vec![],
            request_body: None,
            usage_log_env: Some("GEMINI_USAGE_LOG"),
        },
        ProviderTemplate {
            id: "mistral",
            label: "Mistral",
            env_prefix: "MISTRAL",
            default_endpoint: None,
            method: HttpMethod::Get,
            auth_header: "Authorization",
            auth_mode: AuthMode::Bearer,
            extra_headers: vec![],
            request_body: None,
            usage_log_env: Some("MISTRAL_USAGE_LOG"),
        },
        ProviderTemplate {
            id: "groq",
            label: "Groq",
            env_prefix: "GROQ",
            default_endpoint: None,
            method: HttpMethod::Get,
            auth_header: "Authorization",
            auth_mode: AuthMode::Bearer,
            extra_headers: vec![],
            request_body: None,
            usage_log_env: Some("GROQ_USAGE_LOG"),
        },
        ProviderTemplate {
            id: "copilot",
            label: "Copilot",
            env_prefix: "COPILOT",
            default_endpoint: None,
            method: HttpMethod::Get,
            auth_header: "Authorization",
            auth_mode: AuthMode::Bearer,
            extra_headers: vec![
                ("Accept", "application/vnd.github+json"),
                ("X-GitHub-Api-Version", "2022-11-28"),
            ],
            request_body: None,
            usage_log_env: Some("COPILOT_USAGE_LOG"),
        },
        ProviderTemplate {
            id: "cursor",
            label: "Cursor",
            env_prefix: "CURSOR",
            default_endpoint: Some("https://api.cursor.com/teams/spend"),
            method: HttpMethod::Post,
            auth_header: "Authorization",
            auth_mode: AuthMode::Basic,
            extra_headers: vec![],
            request_body: Some(serde_json::json!({})),
            usage_log_env: Some("CURSOR_USAGE_LOG"),
        },
    ]
}

fn provider_template(provider_id: &str) -> Option<ProviderTemplate> {
    builtin_provider_templates()
        .into_iter()
        .find(|template| template.id == provider_id)
}

pub fn provider_catalog() -> Vec<ProviderCatalogEntry> {
    builtin_provider_templates()
        .into_iter()
        .filter(|template| template.default_endpoint.is_some())
        .map(|template| ProviderCatalogEntry {
            id: template.id.to_string(),
            label: template.label.to_string(),
        })
        .collect()
}

fn keyring_entry(provider_id: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("provider.{provider_id}.api_key"),
    )?)
}

pub fn set_provider_api_key(provider_id: &str, key: Option<&str>) -> Result<()> {
    let mut payload = load_secret_payload();
    match key.map(str::trim) {
        Some(value) if !value.is_empty() => {
            payload
                .provider_api_keys
                .insert(provider_id.to_string(), value.to_string());
        }
        _ => {
            payload.provider_api_keys.remove(provider_id);
        }
    }
    save_secret_payload(&payload)
}

pub fn get_provider_api_key(provider_id: &str) -> Option<String> {
    SecretStore::load()
        .ok()
        .and_then(|payload| payload.provider_api_keys.get(provider_id).cloned())
        .filter(|value| is_non_empty(value))
}

pub fn has_provider_api_key(provider_id: &str) -> bool {
    get_provider_api_key(provider_id).is_some()
}

pub fn set_provider_account_api_key(account_id: &str, key: Option<&str>) -> Result<()> {
    set_provider_api_key(account_id, key)
}

pub fn get_provider_account_api_key(account_id: &str) -> Option<String> {
    get_provider_api_key(account_id)
}

pub fn has_provider_account_api_key(account_id: &str) -> bool {
    get_provider_account_api_key(account_id).is_some()
}

fn openai_session() -> &'static Mutex<OpenAiSessionState> {
    static SESSION: OnceLock<Mutex<OpenAiSessionState>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(OpenAiSessionState::default()))
}

fn is_non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn load_secret_payload() -> SecretPayload {
    SecretStore::load_or_default()
}

fn save_secret_payload(payload: &SecretPayload) -> Result<()> {
    SecretStore::save(payload)
}

// --- OpenAI OAuth token storage (legacy migration helpers) ---

fn oauth_tokens_path() -> Result<PathBuf> {
    Ok(app_config_dir()?.join("oauth_tokens.json"))
}

fn oauth_tokens_entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, "openai.oauth.tokens")?)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OAuthTokenFile {
    #[serde(default)]
    openai_access_token: String,
    #[serde(default)]
    openai_refresh_token: String,
    #[serde(default)]
    openai_account_id: String,
    #[serde(default)]
    openai_plan_type: String,
}

fn load_legacy_oauth_file() -> OAuthTokenFile {
    let path = match oauth_tokens_path() {
        Ok(p) => p,
        Err(_) => return OAuthTokenFile::default(),
    };
    if !path.exists() {
        return OAuthTokenFile::default();
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

fn oauth_tokens_present(file: &OAuthTokenFile) -> bool {
    !file.openai_access_token.is_empty()
        || !file.openai_refresh_token.is_empty()
        || !file.openai_account_id.is_empty()
        || !file.openai_plan_type.is_empty()
}

fn load_legacy_keyring_oauth_tokens() -> OAuthTokenFile {
    let entry = match oauth_tokens_entry() {
        Ok(entry) => entry,
        Err(_) => return OAuthTokenFile::default(),
    };
    let raw = match entry.get_password() {
        Ok(raw) if !raw.trim().is_empty() => raw,
        _ => return OAuthTokenFile::default(),
    };
    let file = match serde_json::from_str::<OAuthTokenFile>(&raw) {
        Ok(file) if oauth_tokens_present(&file) => file,
        _ => return OAuthTokenFile::default(),
    };

    file
}

fn load_stored_openai_secret() -> OpenAiOAuthSecret {
    load_secret_payload().openai_oauth
}

fn persist_openai_secret(secret: &OpenAiOAuthSecret) -> Result<()> {
    let mut payload = load_secret_payload();
    payload.openai_oauth = secret.clone();
    save_secret_payload(&payload)
}

fn clear_stored_openai_secret() -> Result<()> {
    let mut payload = load_secret_payload();
    payload.openai_oauth = OpenAiOAuthSecret::default();
    if payload.provider_api_keys.is_empty() {
        SecretStore::clear()
    } else {
        save_secret_payload(&payload)
    }
}

fn update_in_memory_oauth_session(
    access_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    refresh_token: Option<String>,
    account_id: Option<String>,
    plan_type: Option<String>,
) {
    let mut session = openai_session().lock().unwrap();
    if let Some(access) = access_token {
        session.access_token = Some(access);
        session.expires_at = expires_at;
    }
    if let Some(refresh) = refresh_token {
        session.refresh_token = if is_non_empty(&refresh) {
            Some(refresh)
        } else {
            None
        };
    }
    if let Some(account_id) = account_id {
        session.account_id = account_id;
    }
    if let Some(plan_type) = plan_type {
        session.plan_type = plan_type;
    }
}

fn clear_in_memory_oauth_session() {
    *openai_session().lock().unwrap() = OpenAiSessionState::default();
}

fn current_cached_access_token() -> Option<String> {
    let session = openai_session().lock().unwrap();
    let expires_at = session.expires_at?;
    if expires_at <= Utc::now() {
        return None;
    }
    session.access_token.clone()
}

fn stored_or_cached_account_id() -> String {
    let session = openai_session().lock().unwrap().clone();
    if is_non_empty(&session.account_id) {
        session.account_id
    } else {
        load_stored_openai_secret().account_id
    }
}

fn token_expiry_from_now(expires_in_secs: Option<i64>) -> DateTime<Utc> {
    let ttl = expires_in_secs
        .unwrap_or(DEFAULT_ACCESS_TOKEN_LIFETIME_SECS)
        .max(ACCESS_TOKEN_EXPIRY_SKEW_SECS + 1);
    Utc::now() + Duration::seconds(ttl - ACCESS_TOKEN_EXPIRY_SKEW_SECS)
}

fn current_refresh_token() -> Option<String> {
    let session_refresh = openai_session().lock().unwrap().refresh_token.clone();
    if let Some(refresh) = session_refresh.filter(|value| is_non_empty(value)) {
        return Some(refresh);
    }

    let stored = load_stored_openai_secret();
    if is_non_empty(&stored.refresh_token) {
        Some(stored.refresh_token)
    } else {
        None
    }
}

pub fn has_openai_oauth_session() -> bool {
    current_refresh_token().is_some() || current_cached_access_token().is_some()
}

pub fn get_openai_oauth_access_token() -> Option<String> {
    current_cached_access_token()
}

pub fn get_openai_oauth_plan_type() -> Option<String> {
    let session_plan = openai_session().lock().unwrap().plan_type.clone();
    if is_non_empty(&session_plan) {
        return Some(session_plan);
    }

    let stored = load_stored_openai_secret();
    if is_non_empty(&stored.plan_type) {
        Some(stored.plan_type)
    } else {
        None
    }
}

pub fn set_openai_oauth_tokens(
    access: &str,
    access_expires_at: DateTime<Utc>,
    refresh: &str,
    account_id: &str,
    plan_type: &str,
) -> Result<()> {
    update_in_memory_oauth_session(
        Some(access.to_string()),
        Some(access_expires_at),
        Some(refresh.to_string()),
        Some(account_id.to_string()),
        Some(plan_type.to_string()),
    );

    if !is_non_empty(refresh) {
        return Ok(());
    }

    let mut stored = load_stored_openai_secret();
    stored.refresh_token = refresh.to_string();
    if is_non_empty(account_id) {
        stored.account_id = account_id.to_string();
    }
    if is_non_empty(plan_type) {
        stored.plan_type = plan_type.to_string();
    }
    persist_openai_secret(&stored)
}

pub fn clear_openai_oauth_tokens() {
    clear_in_memory_oauth_session();
    let _ = clear_stored_openai_secret();
    if let Ok(entry) = oauth_tokens_entry() {
        let _ = entry.delete_credential();
    }
    if let Ok(path) = oauth_tokens_path() {
        let _ = fs::remove_file(path);
    }
}

pub fn fetch_openai_oauth_usage() -> Option<UsageSnapshot> {
    if !has_openai_oauth_session() {
        return None;
    }

    let account_id = stored_or_cached_account_id();
    let access_token = match current_cached_access_token() {
        Some(token) => token,
        None => match try_refresh_oauth_token() {
            Ok(token) => token,
            Err(error) => {
                eprintln!("[usageguard] token refresh failed: {error}");
                return Some(error_snapshot(
                    "openai",
                    "ChatGPT",
                    "oauth",
                    Some("oauth_reauth_required"),
                    Some("ChatGPT sign-in expired. Sign in again."),
                ));
            }
        },
    };

    match do_fetch_wham_usage(&access_token, &account_id) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            eprintln!("[usageguard] wham/usage failed: {error}");
            // Token may be expired — try to refresh once
            match try_refresh_oauth_token() {
                Ok(new_access) => {
                    match do_fetch_wham_usage(&new_access, &stored_or_cached_account_id()) {
                        Ok(snapshot) => Some(snapshot),
                        Err(refresh_error) => {
                            eprintln!(
                                "[usageguard] wham/usage after refresh failed: {refresh_error}"
                            );
                            Some(error_snapshot(
                                "openai",
                                "ChatGPT",
                                "oauth",
                                Some("oauth_usage_unavailable"),
                                Some("Unable to load ChatGPT usage right now."),
                            ))
                        }
                    }
                }
                Err(refresh_error) => {
                    eprintln!(
                        "[usageguard] token refresh failed after usage error: {refresh_error}"
                    );
                    Some(error_snapshot(
                        "openai",
                        "ChatGPT",
                        "oauth",
                        Some("oauth_reauth_required"),
                        Some("ChatGPT sign-in expired. Sign in again."),
                    ))
                }
            }
        }
    }
}

fn try_refresh_oauth_token() -> Result<String> {
    let refresh_token =
        current_refresh_token().ok_or_else(|| anyhow!("No refresh token stored"))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", "app_EMoamEEZ73f0CkXaXp7hrann"),
    ];

    let resp = client
        .post("https://auth.openai.com/oauth/token")
        .form(&params)
        .send();
    let resp: Value = match resp {
        Ok(resp) => resp.error_for_status()?.json()?,
        Err(error) => {
            clear_openai_oauth_tokens();
            return Err(error.into());
        }
    };

    let new_access = resp["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("No access_token in refresh response"))?
        .to_string();
    let expires_at = token_expiry_from_now(resp["expires_in"].as_i64());
    let new_refresh = resp["refresh_token"]
        .as_str()
        .map(str::to_string)
        .unwrap_or(refresh_token);
    let mut stored = load_stored_openai_secret();
    stored.refresh_token = new_refresh.clone();
    update_in_memory_oauth_session(
        Some(new_access.clone()),
        Some(expires_at),
        Some(new_refresh),
        Some(stored.account_id.clone()),
        Some(stored.plan_type.clone()),
    );
    if let Err(error) = persist_openai_secret(&stored) {
        clear_in_memory_oauth_session();
        return Err(error);
    }

    Ok(new_access)
}

fn do_fetch_wham_usage(access_token: &str, account_id: &str) -> Result<UsageSnapshot> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;

    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "opencode/0.1");

    if !account_id.is_empty() {
        req = req.header("ChatGPT-Account-Id", account_id);
    }

    let resp = req.send()?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {status}"));
    }
    let value: Value = resp.json()?;
    parse_wham_usage_response(&value)
}

fn parse_wham_usage_response(value: &Value) -> Result<UsageSnapshot> {
    let plan_type = value["plan_type"].as_str().unwrap_or("unknown").to_string();
    let account_id = value["account_id"]
        .as_str()
        .or_else(|| value["user_id"].as_str())
        .unwrap_or_default()
        .to_string();

    let mut stored = load_stored_openai_secret();
    if is_non_empty(&account_id) {
        stored.account_id = account_id.clone();
    }
    if is_non_empty(&plan_type) {
        stored.plan_type = plan_type.clone();
    }
    let _ = persist_openai_secret(&stored);
    update_in_memory_oauth_session(
        None,
        None,
        None,
        Some(stored.account_id.clone()),
        Some(stored.plan_type.clone()),
    );

    // primary_window  = shorter window (e.g. 5 hours)  → maps to the "5h" ring
    // secondary_window = longer window (e.g. 1 week)   → maps to the "week" ring
    // Try nested path first (/rate_limit/…), then flat path (/primary_window/…)
    let primary_percent = value
        .pointer("/rate_limit/primary_window/used_percent")
        .or_else(|| value.pointer("/primary_window/used_percent"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let secondary_percent = value
        .pointer("/rate_limit/secondary_window/used_percent")
        .or_else(|| value.pointer("/secondary_window/used_percent"))
        .and_then(|v| v.as_f64())
        .unwrap_or(primary_percent); // fall back to primary if secondary is absent/null

    // Capitalise first letter: "pro" → "Pro"
    let plan_display: String = {
        let mut chars = plan_type.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };

    // Ring encoding for oauth snapshots (read back in WidgetView fiveHourRatio /
    // weekRatio).  We store both windows as plain 0–100 percentages:
    //   spent_usd / limit_usd(100) = secondary %  → week ring (outer/right)
    //   tokens_in                  = primary %     → 5h ring  (inner/left)
    Ok(UsageSnapshot {
        provider: "openai".into(),
        account_label: format!("ChatGPT {plan_display}"),
        spent_usd: secondary_percent, // week ring = secondary / 100
        limit_usd: 100.0,
        tokens_in: primary_percent.round() as u64, // 5h ring reads this directly
        tokens_out: 0,
        inactive_hours: 0,
        source: "oauth".to_string(),
        status_code: None,
        status_message: None,
    })
}

fn resolve_provider_api_key(provider_id: &str, env_var: &str) -> Option<String> {
    get_provider_api_key(provider_id).or_else(|| std::env::var(env_var).ok())
}

struct ProviderSpec<'a> {
    id: &'a str,
    label: &'a str,
    env_prefix: &'a str,
    api_key: Option<String>,
    endpoint: Option<String>,
    default_endpoint: Option<&'a str>,
    method: HttpMethod,
    auth_header: &'a str,
    auth_mode: AuthMode,
    extra_headers: Vec<(&'a str, String)>,
    request_body: Option<Value>,
    usage_log_env: Option<&'a str>,
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_config_dir()?.join("config.json"))
}

fn legacy_endpoint(cfg: &ApiCredentials, provider_id: &str) -> Option<String> {
    match provider_id {
        "openai" => cfg.openai_costs_endpoint.clone(),
        "anthropic" => cfg.anthropic_costs_endpoint.clone(),
        "gemini" => cfg.gemini_costs_endpoint.clone(),
        "mistral" => cfg.mistral_costs_endpoint.clone(),
        "groq" => cfg.groq_costs_endpoint.clone(),
        "copilot" => cfg.copilot_costs_endpoint.clone(),
        "cursor" => cfg.cursor_costs_endpoint.clone(),
        _ => None,
    }
}

fn clear_legacy_endpoint(cfg: &mut ApiCredentials, provider_id: &str) {
    match provider_id {
        "openai" => cfg.openai_costs_endpoint = None,
        "anthropic" => cfg.anthropic_costs_endpoint = None,
        "gemini" => cfg.gemini_costs_endpoint = None,
        "mistral" => cfg.mistral_costs_endpoint = None,
        "groq" => cfg.groq_costs_endpoint = None,
        "copilot" => cfg.copilot_costs_endpoint = None,
        "cursor" => cfg.cursor_costs_endpoint = None,
        _ => {}
    }
}

fn keyring_password(id: &str) -> Option<String> {
    let entry = keyring_entry(id).ok()?;
    match entry.get_password() {
        Ok(value) if is_non_empty(&value) => Some(value),
        _ => None,
    }
}

fn delete_keyring_password(id: &str) {
    if let Ok(entry) = keyring_entry(id) {
        let _ = entry.delete_credential();
    }
}

fn migrate_secret_payload(cfg: &mut AppConfig) -> Result<bool> {
    let mut payload = load_secret_payload();
    let mut changed = false;
    let mut cleanup_needed = false;
    let mut migrated_keyring_ids = Vec::new();

    for (provider_id, key_slot) in [
        ("openai", &mut cfg.api.openai_api_key),
        ("anthropic", &mut cfg.api.anthropic_api_key),
        ("gemini", &mut cfg.api.gemini_api_key),
        ("mistral", &mut cfg.api.mistral_api_key),
        ("groq", &mut cfg.api.groq_api_key),
        ("copilot", &mut cfg.api.copilot_api_key),
        ("cursor", &mut cfg.api.cursor_api_key),
    ] {
        if let Some(value) = key_slot.take().filter(|value| is_non_empty(value)) {
            payload
                .provider_api_keys
                .insert(provider_id.to_string(), value);
            changed = true;
        }

        if let Some(value) = keyring_password(provider_id) {
            cleanup_needed = true;
            let needs_update = payload.provider_api_keys.get(provider_id) != Some(&value);
            payload
                .provider_api_keys
                .insert(provider_id.to_string(), value);
            if needs_update {
                changed = true;
            }
            migrated_keyring_ids.push(provider_id.to_string());
        }
    }

    for account in &cfg.provider_accounts {
        if let Some(value) = keyring_password(&account.id) {
            cleanup_needed = true;
            let needs_update = payload.provider_api_keys.get(&account.id) != Some(&value);
            payload.provider_api_keys.insert(account.id.clone(), value);
            if needs_update {
                changed = true;
            }
            migrated_keyring_ids.push(account.id.clone());
        }
    }

    let legacy_oauth = {
        let file = load_legacy_oauth_file();
        if oauth_tokens_present(&file) {
            Some(file)
        } else {
            let keyring = load_legacy_keyring_oauth_tokens();
            oauth_tokens_present(&keyring).then_some(keyring)
        }
    };

    if let Some(legacy) = legacy_oauth {
        cleanup_needed = true;
        if is_non_empty(&legacy.openai_refresh_token) {
            if payload.openai_oauth.refresh_token != legacy.openai_refresh_token {
                changed = true;
            }
            payload.openai_oauth.refresh_token = legacy.openai_refresh_token;
        }
        if is_non_empty(&legacy.openai_account_id) {
            if payload.openai_oauth.account_id != legacy.openai_account_id {
                changed = true;
            }
            payload.openai_oauth.account_id = legacy.openai_account_id;
        }
        if is_non_empty(&legacy.openai_plan_type) {
            if payload.openai_oauth.plan_type != legacy.openai_plan_type {
                changed = true;
            }
            payload.openai_oauth.plan_type = legacy.openai_plan_type;
        }
    }

    if !changed && !cleanup_needed {
        return Ok(false);
    }

    if changed {
        save_secret_payload(&payload)?;
    }

    for key_id in migrated_keyring_ids {
        delete_keyring_password(&key_id);
    }
    if let Ok(entry) = oauth_tokens_entry() {
        let _ = entry.delete_credential();
    }
    if let Ok(path) = oauth_tokens_path() {
        let _ = fs::remove_file(path);
    }

    Ok(true)
}

fn migrate_legacy_provider_accounts(cfg: &mut AppConfig) -> bool {
    if !cfg.provider_accounts.is_empty() {
        return false;
    }

    let mut migrated = false;
    for template in builtin_provider_templates() {
        if template.default_endpoint.is_none() {
            continue;
        }

        let endpoint = legacy_endpoint(&cfg.api, template.id);
        let legacy_key = get_provider_api_key(template.id);
        if endpoint.is_none() && legacy_key.is_none() {
            continue;
        }

        let account_id = format!("acct_{}_default", template.id);
        if let Some(key) = legacy_key {
            let _ = set_provider_account_api_key(&account_id, Some(&key));
            let _ = set_provider_api_key(template.id, None);
        }

        cfg.provider_accounts.push(ProviderAccount {
            id: account_id,
            provider: template.id.to_string(),
            label: template.label.to_string(),
            endpoint: None,
        });
        clear_legacy_endpoint(&mut cfg.api, template.id);
        migrated = true;
    }

    migrated
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read config file: {}", path.display()))?;
    let mut cfg = serde_json::from_str::<AppConfig>(&raw)
        .with_context(|| format!("Invalid config JSON: {}", path.display()))?;

    let mut migrated = false;
    migrated |= migrate_secret_payload(&mut cfg)?;
    migrated |= migrate_legacy_provider_accounts(&mut cfg);

    if !cfg.profiles.is_empty() {
        cfg.profiles.clear();
        migrated = true;
    }

    for account in &mut cfg.provider_accounts {
        if account.endpoint.take().is_some() {
            migrated = true;
        }
    }

    for provider_id in [
        "openai",
        "anthropic",
        "gemini",
        "mistral",
        "groq",
        "copilot",
        "cursor",
    ] {
        if legacy_endpoint(&cfg.api, provider_id).is_some() {
            clear_legacy_endpoint(&mut cfg.api, provider_id);
            migrated = true;
        }
    }

    let before_accounts = cfg.provider_accounts.len();
    cfg.provider_accounts.retain(|account| {
        provider_template(&account.provider)
            .and_then(|template| template.default_endpoint)
            .is_some()
    });
    if cfg.provider_accounts.len() != before_accounts {
        migrated = true;
    }

    if migrated {
        save_config(&cfg)?;
    }

    Ok(cfg)
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    let path = config_path()?;
    let dir = path
        .parent()
        .context("Config parent directory missing")?
        .to_path_buf();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Unable to create config dir: {}", dir.display()))?;
    let raw = serde_json::to_string_pretty(cfg)?;
    fs::write(&path, raw)
        .with_context(|| format!("Unable to write config file: {}", path.display()))?;
    Ok(())
}

pub fn evaluate_alerts(snapshot: &UsageSnapshot, cfg: &AppConfig) -> Vec<Alert> {
    let mut alerts = vec![];
    let ratio = if snapshot.limit_usd > 0.0 {
        snapshot.spent_usd / snapshot.limit_usd
    } else {
        0.0
    };

    if snapshot.limit_usd > 0.0 && ratio >= 1.0 {
        alerts.push(Alert {
            level: "critical".into(),
            code: "limit_exceeded".into(),
            message: format!(
                "Budget exceeded: ${:.2} / ${:.2}",
                snapshot.spent_usd, snapshot.limit_usd
            ),
        });
    } else if snapshot.limit_usd > 0.0 && ratio >= cfg.near_limit_ratio {
        alerts.push(Alert {
            level: "warning".into(),
            code: "near_limit".into(),
            message: format!(
                "Near budget limit: ${:.2} / ${:.2}",
                snapshot.spent_usd, snapshot.limit_usd
            ),
        });
    }

    if snapshot.inactive_hours >= cfg.inactive_threshold_hours {
        alerts.push(Alert {
            level: "info".into(),
            code: "under_used".into(),
            message: format!("Low usage: no activity for {}h", snapshot.inactive_hours),
        });
    }

    alerts
}

pub fn is_quiet_hour(now: DateTime<Local>, quiet: &QuietHours) -> bool {
    if !quiet.enabled {
        return false;
    }
    let h = now.hour() as u8;
    if quiet.start_hour == quiet.end_hour {
        return false;
    }
    if quiet.start_hour < quiet.end_hour {
        h >= quiet.start_hour && h < quiet.end_hour
    } else {
        h >= quiet.start_hour || h < quiet.end_hour
    }
}

pub fn should_notify(alerts: &[Alert], now: DateTime<Local>, cfg: &AppConfig) -> bool {
    if alerts.is_empty() {
        return false;
    }
    let has_critical = alerts.iter().any(|a| a.level == "critical");
    has_critical || !is_quiet_hour(now, &cfg.quiet_hours)
}

fn build_legacy_provider_specs() -> Vec<ProviderSpec<'static>> {
    builtin_provider_templates()
        .into_iter()
        .map(|template| ProviderSpec {
            id: template.id,
            label: template.label,
            env_prefix: template.env_prefix,
            api_key: match template.id {
                "openai" => resolve_provider_api_key("openai", "OPENAI_API_KEY"),
                "anthropic" => resolve_provider_api_key("anthropic", "ANTHROPIC_API_KEY"),
                "gemini" => resolve_provider_api_key("gemini", "GEMINI_API_KEY"),
                "mistral" => resolve_provider_api_key("mistral", "MISTRAL_API_KEY"),
                "groq" => resolve_provider_api_key("groq", "GROQ_API_KEY"),
                "copilot" => resolve_provider_api_key("copilot", "COPILOT_API_KEY"),
                "cursor" => resolve_provider_api_key("cursor", "CURSOR_API_KEY"),
                _ => None,
            },
            endpoint: None,
            default_endpoint: template.default_endpoint,
            method: template.method.clone(),
            auth_header: template.auth_header,
            auth_mode: template.auth_mode,
            extra_headers: template
                .extra_headers
                .iter()
                .map(|(key, value)| (*key, (*value).to_string()))
                .collect(),
            request_body: template.request_body.clone(),
            usage_log_env: template.usage_log_env,
        })
        .collect()
}

fn build_provider_account_spec(account: &ProviderAccount) -> Option<ProviderSpec<'_>> {
    let template = provider_template(&account.provider)?;
    template.default_endpoint?;
    Some(ProviderSpec {
        id: template.id,
        label: &account.label,
        env_prefix: template.env_prefix,
        api_key: get_provider_account_api_key(&account.id),
        endpoint: None,
        default_endpoint: template.default_endpoint,
        method: template.method.clone(),
        auth_header: template.auth_header,
        auth_mode: template.auth_mode,
        extra_headers: template
            .extra_headers
            .iter()
            .map(|(key, value)| (*key, (*value).to_string()))
            .collect(),
        request_body: template.request_body.clone(),
        usage_log_env: None,
    })
}

pub fn provider_snapshots(cfg: &AppConfig) -> Vec<UsageSnapshot> {
    let mut items: Vec<UsageSnapshot> = vec![];

    // OAuth subscriptions first (ChatGPT Plus/Pro via wham/usage)
    if let Some(s) = fetch_openai_oauth_usage() {
        items.push(s);
    }

    // API-key / env sources
    let api_items: Vec<UsageSnapshot> = if cfg.provider_accounts.is_empty() {
        build_legacy_provider_specs()
            .into_iter()
            .filter_map(fetch_provider_snapshot)
            .collect()
    } else {
        cfg.provider_accounts
            .iter()
            .filter_map(build_provider_account_spec)
            .filter_map(fetch_provider_snapshot)
            .collect()
    };
    items.extend(api_items);

    items
}

fn fetch_provider_snapshot(spec: ProviderSpec<'_>) -> Option<UsageSnapshot> {
    if let Some(log_env) = spec.usage_log_env {
        if let Ok(path) = std::env::var(log_env) {
            if let Ok(s) = snapshot_from_ndjson(&path, spec.id, spec.label) {
                return Some(s);
            }
        }
    }

    let endpoint = spec
        .endpoint
        .or_else(|| spec.default_endpoint.map(|v| v.to_string()));

    if let (Some(url), Some(key)) = (endpoint, spec.api_key) {
        match snapshot_from_http_json(
            &url,
            spec.method,
            Some((spec.auth_header, spec.auth_mode, key.as_str())),
            &spec.extra_headers,
            spec.request_body.as_ref(),
            spec.id,
            spec.label,
            "api",
        ) {
            Ok(s) => return Some(s),
            Err(_error) => {
                return Some(error_snapshot(
                    spec.id,
                    spec.label,
                    "api",
                    Some("api_usage_unavailable"),
                    Some("Unable to load provider usage right now."),
                ));
            }
        }
    }

    env_fallback_snapshot(spec.id, spec.label, spec.env_prefix)
}

fn snapshot_from_http_json(
    url: &str,
    method: HttpMethod,
    auth: Option<(&str, AuthMode, &str)>,
    headers: &[(&str, String)],
    request_body: Option<&Value>,
    provider: &str,
    label: &str,
    source: &str,
) -> Result<UsageSnapshot> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;

    let mut req = match method {
        HttpMethod::Get => client.get(url),
        HttpMethod::Post => client.post(url),
    };
    if let Some((header, auth_mode, key)) = auth {
        req = apply_auth(req, header, auth_mode, key);
    }
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    if let Some(body) = request_body {
        req = req.json(body);
    }

    let res = req.send()?.error_for_status()?;
    let value: Value = res.json()?;

    // strict-ish known responses first
    if provider == "openai" {
        if let Ok(s) = parse_openai_costs_response(&value, label, source) {
            return Ok(s);
        }
    }
    if provider == "anthropic" {
        if let Ok(s) = parse_anthropic_usage_response(&value, label, source) {
            return Ok(s);
        }
    }
    if provider == "copilot" {
        if let Ok(s) = parse_copilot_usage_response(&value, label, source) {
            return Ok(s);
        }
    }
    if provider == "cursor" {
        if let Ok(s) = parse_cursor_spend_response(&value, label, source) {
            return Ok(s);
        }
    }

    snapshot_from_value(&value, provider, label, source)
}

fn apply_auth(
    req: reqwest::blocking::RequestBuilder,
    header: &str,
    auth_mode: AuthMode,
    key: &str,
) -> reqwest::blocking::RequestBuilder {
    match auth_mode {
        AuthMode::Bearer if header.eq_ignore_ascii_case("authorization") => req.bearer_auth(key),
        AuthMode::Bearer => req.header(header, format!("Bearer {key}")),
        AuthMode::Raw => req.header(header, key),
        AuthMode::Basic if header.eq_ignore_ascii_case("authorization") => {
            req.basic_auth(key, Some(""))
        }
        AuthMode::Basic => req.header(header, key),
    }
}

fn parse_openai_costs_response(value: &Value, label: &str, source: &str) -> Result<UsageSnapshot> {
    let spent_usd = pick_f64(
        value,
        &["total_spent_usd", "spent_usd", "spent", "cost_usd"],
    )
    .or_else(|| {
        value.get("data").and_then(|d| d.as_array()).map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    r.get("amount")
                        .and_then(|a| a.get("value"))
                        .and_then(|v| v.as_f64())
                        .or_else(|| pick_f64(r, &["cost_usd", "spent_usd", "amount_usd"]))
                })
                .sum::<f64>()
        })
    })
    .unwrap_or(0.0);

    Ok(UsageSnapshot {
        provider: "openai".into(),
        account_label: label.to_string(),
        spent_usd,
        limit_usd: pick_f64(value, &["limit_usd", "budget_usd", "hard_limit_usd"]).unwrap_or(0.0),
        tokens_in: pick_u64(value, &["tokens_in", "input_tokens", "total_input_tokens"])
            .unwrap_or(0),
        tokens_out: pick_u64(
            value,
            &["tokens_out", "output_tokens", "total_output_tokens"],
        )
        .unwrap_or(0),
        inactive_hours: derive_inactive_hours(value),
        source: source.to_string(),
        status_code: None,
        status_message: None,
    })
}

fn parse_anthropic_usage_response(
    value: &Value,
    label: &str,
    source: &str,
) -> Result<UsageSnapshot> {
    let rows = value
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let spent_rows_sum = rows
        .iter()
        .filter_map(|r| pick_f64(r, &["cost_usd", "amount_usd", "spent_usd"]))
        .sum::<f64>();

    let spent_usd =
        pick_f64(value, &["total_cost_usd", "spent_usd", "cost_usd"]).unwrap_or(spent_rows_sum);

    Ok(UsageSnapshot {
        provider: "anthropic".into(),
        account_label: label.to_string(),
        spent_usd,
        limit_usd: pick_f64(value, &["limit_usd", "budget_usd"]).unwrap_or(0.0),
        tokens_in: pick_u64(value, &["tokens_in", "input_tokens", "total_input_tokens"])
            .unwrap_or_else(|| {
                rows.iter()
                    .filter_map(|r| pick_u64(r, &["input_tokens", "tokens_in"]))
                    .sum()
            }),
        tokens_out: pick_u64(
            value,
            &["tokens_out", "output_tokens", "total_output_tokens"],
        )
        .unwrap_or_else(|| {
            rows.iter()
                .filter_map(|r| pick_u64(r, &["output_tokens", "tokens_out"]))
                .sum()
        }),
        inactive_hours: derive_inactive_hours(value),
        source: source.to_string(),
        status_code: None,
        status_message: None,
    })
}

fn parse_copilot_usage_response(value: &Value, label: &str, source: &str) -> Result<UsageSnapshot> {
    let rows = value
        .get("usageItems")
        .and_then(|items| items.as_array())
        .cloned()
        .unwrap_or_default();

    let spent_usd =
        pick_f64(value, &["spent_usd", "cost_usd", "total_cost_usd"]).unwrap_or_else(|| {
            rows.iter()
                .filter_map(|row| pick_f64(row, &["netAmount", "amount", "amount_usd"]))
                .sum()
        });

    let request_count = rows
        .iter()
        .filter_map(|row| pick_u64(row, &["netQuantity", "quantity", "count"]))
        .sum();

    Ok(UsageSnapshot {
        provider: "copilot".into(),
        account_label: label.to_string(),
        spent_usd,
        limit_usd: pick_f64(value, &["limit_usd", "budget_usd"]).unwrap_or(0.0),
        tokens_in: 0,
        tokens_out: request_count,
        inactive_hours: derive_inactive_hours(value),
        source: source.to_string(),
        status_code: None,
        status_message: None,
    })
}

fn parse_cursor_spend_response(value: &Value, label: &str, source: &str) -> Result<UsageSnapshot> {
    let rows = value
        .get("items")
        .and_then(|items| items.as_array())
        .cloned()
        .unwrap_or_default();

    let spent_usd = pick_f64(value, &["spent_usd", "cost_usd", "total_cost_usd"])
        .or_else(|| pick_f64(value, &["total_cents"]).map(|cents| cents / 100.0))
        .unwrap_or_else(|| {
            rows.iter()
                .filter_map(|row| pick_f64(row, &["total_cents", "cents", "cost_cents"]))
                .map(|cents| cents / 100.0)
                .sum()
        });

    Ok(UsageSnapshot {
        provider: "cursor".into(),
        account_label: label.to_string(),
        spent_usd,
        limit_usd: pick_f64(value, &["limit_usd", "budget_usd"]).unwrap_or(0.0),
        tokens_in: 0,
        tokens_out: 0,
        inactive_hours: derive_inactive_hours(value),
        source: source.to_string(),
        status_code: None,
        status_message: None,
    })
}

fn snapshot_from_ndjson(path: &str, provider: &str, label: &str) -> Result<UsageSnapshot> {
    let raw = fs::read_to_string(path).with_context(|| format!("Unable to read {path}"))?;
    let mut last: Option<Value> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            last = Some(v);
        }
    }

    let value = last.ok_or_else(|| anyhow!("No valid JSON rows in {path}"))?;
    snapshot_from_value(&value, provider, label, "env")
}

fn snapshot_from_value(
    value: &Value,
    provider: &str,
    label: &str,
    source: &str,
) -> Result<UsageSnapshot> {
    Ok(UsageSnapshot {
        provider: provider.to_string(),
        account_label: label.to_string(),
        spent_usd: pick_f64(value, &["spent_usd", "spent", "cost_usd", "total_cost_usd"])
            .unwrap_or(0.0),
        limit_usd: pick_f64(
            value,
            &["limit_usd", "budget_usd", "limit", "hard_limit_usd"],
        )
        .unwrap_or(0.0),
        tokens_in: pick_u64(value, &["tokens_in", "input_tokens", "total_input_tokens"])
            .unwrap_or(0),
        tokens_out: pick_u64(
            value,
            &["tokens_out", "output_tokens", "total_output_tokens"],
        )
        .unwrap_or(0),
        inactive_hours: derive_inactive_hours(value),
        source: source.to_string(),
        status_code: None,
        status_message: None,
    })
}

fn derive_inactive_hours(value: &Value) -> u32 {
    if let Some(h) = pick_u64(value, &["inactive_hours"]) {
        h as u32
    } else if let Some(ts) = pick_str(value, &["last_activity_iso", "last_activity", "timestamp"]) {
        inactive_hours_from_iso(ts).unwrap_or(0)
    } else {
        0
    }
}

fn env_fallback_snapshot(provider: &str, label: &str, prefix: &str) -> Option<UsageSnapshot> {
    let spent = std::env::var(format!("{prefix}_SPENT_USD"))
        .ok()
        .and_then(|v| v.parse::<f64>().ok());
    let limit = std::env::var(format!("{prefix}_LIMIT_USD"))
        .ok()
        .and_then(|v| v.parse::<f64>().ok());

    if spent.is_none() && limit.is_none() {
        return None;
    }

    Some(UsageSnapshot {
        provider: provider.to_string(),
        account_label: label.to_string(),
        spent_usd: spent.unwrap_or(0.0),
        limit_usd: limit.unwrap_or(0.0),
        tokens_in: 0,
        tokens_out: 0,
        inactive_hours: 0,
        source: "env".to_string(),
        status_code: None,
        status_message: None,
    })
}

fn error_snapshot(
    provider: &str,
    label: &str,
    source: &str,
    status_code: Option<&str>,
    status_message: Option<&str>,
) -> UsageSnapshot {
    UsageSnapshot {
        provider: provider.to_string(),
        account_label: label.to_string(),
        spent_usd: 0.0,
        limit_usd: 0.0,
        tokens_in: 0,
        tokens_out: 0,
        inactive_hours: 0,
        source: source.to_string(),
        status_code: status_code.map(str::to_string),
        status_message: status_message.map(str::to_string),
    }
}

fn pick_f64(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        v.get(*k).and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_u64().map(|n| n as f64))
                .or_else(|| x.as_i64().map(|n| n as f64))
        })
    })
}

fn pick_u64(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_u64()))
}

fn pick_str<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()))
}

fn inactive_hours_from_iso(ts: &str) -> Option<u32> {
    let parsed = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
    let now = Utc::now();
    let delta = now.signed_duration_since(parsed);
    Some(delta.num_hours().max(0) as u32)
}

pub fn demo_snapshots() -> Vec<UsageSnapshot> {
    vec![
        UsageSnapshot {
            provider: "openai".into(),
            account_label: "OpenAI".into(),
            spent_usd: 12.4,
            limit_usd: 30.0,
            tokens_in: 184_000,
            tokens_out: 12_300,
            inactive_hours: 2,
            source: "demo".into(),
            status_code: None,
            status_message: None,
        },
        UsageSnapshot {
            provider: "anthropic".into(),
            account_label: "Anthropic".into(),
            spent_usd: 6.7,
            limit_usd: 20.0,
            tokens_in: 92_000,
            tokens_out: 8_400,
            inactive_hours: 11,
            source: "demo".into(),
            status_code: None,
            status_message: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_limit_alert() {
        let cfg = AppConfig::default();
        let s = UsageSnapshot {
            provider: "x".into(),
            account_label: "y".into(),
            spent_usd: 9.0,
            limit_usd: 10.0,
            tokens_in: 0,
            tokens_out: 0,
            inactive_hours: 1,
            source: "test".into(),
            status_code: None,
            status_message: None,
        };
        let alerts = evaluate_alerts(&s, &cfg);
        assert!(alerts.iter().any(|a| a.code == "near_limit"));
    }

    #[test]
    fn parse_flexible_json_shape() {
        let value: Value = serde_json::json!({
            "spent": 5.5,
            "budget_usd": 20.0,
            "input_tokens": 111,
            "output_tokens": 222,
            "inactive_hours": 3
        });

        let snap = snapshot_from_value(&value, "openai", "OpenAI", "api").unwrap();
        assert_eq!(snap.spent_usd, 5.5);
        assert_eq!(snap.limit_usd, 20.0);
        assert_eq!(snap.tokens_in, 111);
        assert_eq!(snap.tokens_out, 222);
        assert_eq!(snap.inactive_hours, 3);
    }

    #[test]
    fn parse_copilot_usage_rows() {
        let value: Value = serde_json::json!({
            "usageItems": [
                { "netAmount": 1.25, "netQuantity": 10 },
                { "netAmount": 2.75, "netQuantity": 30 }
            ]
        });

        let snap = parse_copilot_usage_response(&value, "Copilot", "api").unwrap();
        assert_eq!(snap.spent_usd, 4.0);
        assert_eq!(snap.tokens_in, 0);
        assert_eq!(snap.tokens_out, 40);
    }

    #[test]
    fn parse_cursor_total_cents() {
        let value: Value = serde_json::json!({
            "total_cents": 1234
        });

        let snap = parse_cursor_spend_response(&value, "Cursor", "api").unwrap();
        assert!((snap.spent_usd - 12.34).abs() < f64::EPSILON);
    }
}
