mod secret_store;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use secret_store::{
    app_config_dir, AnthropicOAuthSecret, OpenAiOAuthSecret, SecretPayload, SecretStore,
};

const KEYRING_SERVICE: &str = "usage-guard";
const ACCESS_TOKEN_EXPIRY_SKEW_SECS: i64 = 60;
const DEFAULT_ACCESS_TOKEN_LIFETIME_SECS: i64 = 45 * 60;
const ANTHROPIC_OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";
const ANTHROPIC_OAUTH_SCOPE: &str =
    "user:inference user:mcp_servers user:profile user:sessions:claude_code";
const CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV: &str = "USAGEGUARD_CLAUDE_CREDENTIALS_PATH_OVERRIDE";
pub const DEFAULT_REFRESH_INTERVAL_SECS: u32 = 15;
pub const MIN_REFRESH_INTERVAL_SECS: u32 = 15;
pub const MAX_REFRESH_INTERVAL_SECS: u32 = 900;

fn default_refresh_interval_secs() -> u32 {
    DEFAULT_REFRESH_INTERVAL_SECS
}

pub fn clamp_refresh_interval_secs(value: u32) -> u32 {
    value.clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS)
}

#[derive(Debug, Clone, Default)]
struct OpenAiSessionState {
    access_token: Option<String>,
    refresh_token: Option<String>,
    account_id: String,
    plan_type: String,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
struct AnthropicSessionState {
    access_token: Option<String>,
    refresh_token: Option<String>,
    subscription_type: String,
    rate_limit_tier: String,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ClaudeDesktopCredentials {
    #[serde(default, rename = "claudeAiOauth")]
    claude_ai_oauth: ClaudeDesktopOAuth,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ClaudeDesktopOAuth {
    #[serde(default, rename = "subscriptionType")]
    subscription_type: String,
    #[serde(default, rename = "rateLimitTier")]
    rate_limit_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiMetricWindow {
    #[serde(default)]
    pub spend_usd: f64,
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub requests: Option<u64>,
}

impl Default for ApiMetricWindow {
    fn default() -> Self {
        Self {
            spend_usd: 0.0,
            tokens_in: 0,
            tokens_out: 0,
            requests: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiMetricCard {
    #[serde(default)]
    pub today: ApiMetricWindow,
    #[serde(default)]
    pub rolling_30d: ApiMetricWindow,
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
    #[serde(default)]
    pub api_metrics: Option<ApiMetricCard>,
    #[serde(default)]
    pub primary_reset_at: Option<String>,
    #[serde(default)]
    pub secondary_reset_at: Option<String>,
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

    pub openai_costs_endpoint: Option<String>,
    pub anthropic_costs_endpoint: Option<String>,
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
    #[serde(default = "default_refresh_interval_secs")]
    pub refresh_interval_secs: u32,
    pub api: ApiCredentials,
    #[serde(default)]
    pub provider_accounts: Vec<ProviderAccount>,
    #[serde(default)]
    pub profiles: Vec<ProviderProfile>,
    /// Last known widget position in logical pixels [x, y].
    /// Saved on quit and restored on next launch.
    #[serde(default)]
    pub widget_position: Option<[f64; 2]>,
    /// User-defined display name for the ChatGPT OAuth subscription connection.
    #[serde(default)]
    pub openai_oauth_label: Option<String>,
    /// User-defined display name for the Claude OAuth subscription connection.
    #[serde(default)]
    pub anthropic_oauth_label: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            near_limit_ratio: 0.85,
            inactive_threshold_hours: 8,
            quiet_hours: QuietHours::default(),
            refresh_interval_secs: DEFAULT_REFRESH_INTERVAL_SECS,
            api: ApiCredentials::default(),
            provider_accounts: vec![],
            profiles: vec![],
            widget_position: None,
            openai_oauth_label: None,
            anthropic_oauth_label: None,
        }
    }
}

#[derive(Clone, Copy)]
enum HttpMethod {
    Get,
}

#[derive(Clone, Copy)]
enum AuthMode {
    Bearer,
    Raw,
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
            default_endpoint: Some(
                "https://api.anthropic.com/v1/organizations/usage_report/messages",
            ),
            method: HttpMethod::Get,
            auth_header: "x-api-key",
            auth_mode: AuthMode::Raw,
            extra_headers: vec![("anthropic-version", "2023-06-01")],
            request_body: None,
            usage_log_env: Some("ANTHROPIC_USAGE_LOG"),
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

fn anthropic_session() -> &'static Mutex<AnthropicSessionState> {
    static SESSION: OnceLock<Mutex<AnthropicSessionState>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(AnthropicSessionState::default()))
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

fn has_remaining_secret_payload(payload: &SecretPayload) -> bool {
    !payload.provider_api_keys.is_empty()
        || payload.openai_oauth != OpenAiOAuthSecret::default()
        || payload.anthropic_oauth != AnthropicOAuthSecret::default()
}

fn payload_after_clearing_openai_secret(mut payload: SecretPayload) -> Option<SecretPayload> {
    payload.openai_oauth = OpenAiOAuthSecret::default();
    has_remaining_secret_payload(&payload).then_some(payload)
}

fn payload_after_clearing_anthropic_secret(mut payload: SecretPayload) -> Option<SecretPayload> {
    payload.anthropic_oauth = AnthropicOAuthSecret::default();
    has_remaining_secret_payload(&payload).then_some(payload)
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
    let payload = load_secret_payload();
    if let Some(payload) = payload_after_clearing_openai_secret(payload) {
        save_secret_payload(&payload)
    } else {
        SecretStore::clear()
    }
}

fn load_stored_anthropic_secret() -> AnthropicOAuthSecret {
    load_secret_payload().anthropic_oauth
}

fn persist_anthropic_secret(secret: &AnthropicOAuthSecret) -> Result<()> {
    let mut payload = load_secret_payload();
    payload.anthropic_oauth = secret.clone();
    save_secret_payload(&payload)
}

fn clear_stored_anthropic_secret() -> Result<()> {
    let payload = load_secret_payload();
    if let Some(payload) = payload_after_clearing_anthropic_secret(payload) {
        save_secret_payload(&payload)
    } else {
        SecretStore::clear()
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

fn update_in_memory_anthropic_session(
    access_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    refresh_token: Option<String>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
) {
    let mut session = anthropic_session().lock().unwrap();
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
    if let Some(subscription_type) = subscription_type {
        session.subscription_type = subscription_type;
    }
    if let Some(rate_limit_tier) = rate_limit_tier {
        session.rate_limit_tier = rate_limit_tier;
    }
}

fn clear_in_memory_anthropic_session() {
    *anthropic_session().lock().unwrap() = AnthropicSessionState::default();
}

fn current_cached_anthropic_access_token() -> Option<String> {
    let session = anthropic_session().lock().unwrap();
    let expires_at = session.expires_at?;
    if expires_at <= Utc::now() {
        return None;
    }
    session.access_token.clone()
}

fn current_anthropic_refresh_token() -> Option<String> {
    let session_refresh = anthropic_session().lock().unwrap().refresh_token.clone();
    if let Some(refresh) = session_refresh.filter(|value| is_non_empty(value)) {
        return Some(refresh);
    }

    let stored = load_stored_anthropic_secret();
    if is_non_empty(&stored.refresh_token) {
        Some(stored.refresh_token)
    } else {
        None
    }
}

fn normalize_plan_label(value: &str) -> String {
    value
        .split(|ch: char| ch == '_' || ch == '-' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut normalized = first.to_uppercase().collect::<String>();
                    normalized.push_str(&chars.as_str().to_lowercase());
                    normalized
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn anthropic_plan_label_from_subscription_type(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    match value.to_ascii_lowercase().as_str() {
        "pro" => Some("Pro".to_string()),
        "max" => Some("Max".to_string()),
        "team" => Some("Team".to_string()),
        "enterprise" => Some("Enterprise".to_string()),
        _ => Some(normalize_plan_label(value)),
    }
}

fn anthropic_plan_label_from_rate_limit_tier(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let parts = normalized
        .split(|ch: char| ch == '_' || ch == '-' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return None;
    }
    if parts.iter().any(|part| *part == "enterprise") {
        return Some("Enterprise".to_string());
    }
    if parts.iter().any(|part| *part == "team") {
        return Some("Team".to_string());
    }
    if parts.iter().any(|part| *part == "max") {
        return Some("Max".to_string());
    }
    if parts.iter().any(|part| *part == "pro") {
        return Some("Pro".to_string());
    }

    None
}

fn claude_credentials_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    Some(dirs::home_dir()?.join(".claude").join(".credentials.json"))
}

fn load_local_claude_oauth_metadata() -> Option<(String, String)> {
    let path = claude_credentials_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let credentials = serde_json::from_str::<ClaudeDesktopCredentials>(&raw).ok()?;
    let subscription_type = credentials
        .claude_ai_oauth
        .subscription_type
        .trim()
        .to_string();
    let rate_limit_tier = credentials
        .claude_ai_oauth
        .rate_limit_tier
        .trim()
        .to_string();

    if is_non_empty(&subscription_type) || is_non_empty(&rate_limit_tier) {
        Some((subscription_type, rate_limit_tier))
    } else {
        None
    }
}

fn sync_anthropic_plan_metadata_from_local_credentials() -> Option<(String, String)> {
    let (subscription_type, rate_limit_tier) = load_local_claude_oauth_metadata()?;
    let mut stored = load_stored_anthropic_secret();

    if is_non_empty(&subscription_type) {
        stored.subscription_type = subscription_type.clone();
    }
    if is_non_empty(&rate_limit_tier) {
        stored.rate_limit_tier = rate_limit_tier.clone();
    }

    let _ = persist_anthropic_secret(&stored);
    update_in_memory_anthropic_session(
        None,
        None,
        None,
        is_non_empty(&stored.subscription_type).then_some(stored.subscription_type.clone()),
        is_non_empty(&stored.rate_limit_tier).then_some(stored.rate_limit_tier.clone()),
    );

    Some((stored.subscription_type, stored.rate_limit_tier))
}

fn anthropic_plan_type_from_fields(
    subscription_type: &str,
    rate_limit_tier: &str,
) -> Option<String> {
    anthropic_plan_label_from_subscription_type(subscription_type)
        .or_else(|| anthropic_plan_label_from_rate_limit_tier(rate_limit_tier))
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
        return Some(openai_oauth_plan_label(&session_plan));
    }

    let stored = load_stored_openai_secret();
    if is_non_empty(&stored.plan_type) {
        Some(openai_oauth_plan_label(&stored.plan_type))
    } else {
        None
    }
}

pub fn has_anthropic_oauth_session() -> bool {
    current_anthropic_refresh_token().is_some() || current_cached_anthropic_access_token().is_some()
}

pub fn get_anthropic_oauth_plan_type() -> Option<String> {
    let session = anthropic_session().lock().unwrap().clone();
    if let Some(plan_type) =
        anthropic_plan_type_from_fields(&session.subscription_type, &session.rate_limit_tier)
    {
        return Some(plan_type);
    }

    let stored = load_stored_anthropic_secret();
    anthropic_plan_type_from_fields(&stored.subscription_type, &stored.rate_limit_tier).or_else(
        || {
            let (subscription_type, rate_limit_tier) =
                sync_anthropic_plan_metadata_from_local_credentials()?;
            anthropic_plan_type_from_fields(&subscription_type, &rate_limit_tier)
        },
    )
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

pub fn set_anthropic_oauth_tokens(
    access: &str,
    access_expires_at: DateTime<Utc>,
    refresh: &str,
    subscription_type: &str,
    rate_limit_tier: &str,
) -> Result<()> {
    update_in_memory_anthropic_session(
        Some(access.to_string()),
        Some(access_expires_at),
        Some(refresh.to_string()),
        Some(subscription_type.to_string()),
        Some(rate_limit_tier.to_string()),
    );

    if !is_non_empty(refresh) {
        return Ok(());
    }

    let mut stored = load_stored_anthropic_secret();
    stored.refresh_token = refresh.to_string();
    if is_non_empty(subscription_type) {
        stored.subscription_type = subscription_type.to_string();
    }
    if is_non_empty(rate_limit_tier) {
        stored.rate_limit_tier = rate_limit_tier.to_string();
    }
    persist_anthropic_secret(&stored)
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

pub fn clear_anthropic_oauth_tokens() {
    clear_in_memory_anthropic_session();
    let _ = clear_stored_anthropic_secret();
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

pub fn fetch_anthropic_oauth_usage() -> Option<UsageSnapshot> {
    if !has_anthropic_oauth_session() {
        return None;
    }

    let access_token = match current_cached_anthropic_access_token() {
        Some(token) => token,
        None => match try_refresh_anthropic_oauth_token() {
            Ok(token) => token,
            Err(error) => {
                eprintln!("[usageguard] anthropic token refresh failed: {error}");
                return Some(error_snapshot(
                    "anthropic",
                    "Claude",
                    "oauth",
                    Some("oauth_reauth_required"),
                    Some("Claude sign-in expired. Sign in again."),
                ));
            }
        },
    };

    match do_fetch_anthropic_oauth_usage(&access_token) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            eprintln!("[usageguard] anthropic oauth usage failed: {error}");
            match try_refresh_anthropic_oauth_token() {
                Ok(new_access) => match do_fetch_anthropic_oauth_usage(&new_access) {
                    Ok(snapshot) => Some(snapshot),
                    Err(refresh_error) => {
                        eprintln!(
                            "[usageguard] anthropic oauth usage after refresh failed: {refresh_error}"
                        );
                        Some(error_snapshot(
                            "anthropic",
                            "Claude",
                            "oauth",
                            Some("oauth_usage_unavailable"),
                            Some("Unable to load Claude usage right now."),
                        ))
                    }
                },
                Err(refresh_error) => {
                    eprintln!(
                        "[usageguard] anthropic token refresh failed after usage error: {refresh_error}"
                    );
                    Some(error_snapshot(
                        "anthropic",
                        "Claude",
                        "oauth",
                        Some("oauth_reauth_required"),
                        Some("Claude sign-in expired. Sign in again."),
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

    let resp: Value = client
        .post("https://auth.openai.com/oauth/token")
        .form(&params)
        .send()
        .map_err(|error| {
            clear_openai_oauth_tokens();
            anyhow!(error)
        })?
        .error_for_status()
        .map_err(|error| {
            clear_openai_oauth_tokens();
            anyhow!(error)
        })?
        .json()
        .map_err(|error| {
            clear_openai_oauth_tokens();
            anyhow!(error)
        })?;

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

fn try_refresh_anthropic_oauth_token() -> Result<String> {
    let refresh_token = current_anthropic_refresh_token()
        .ok_or_else(|| anyhow!("No Anthropic refresh token stored"))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let params = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
        "scope": ANTHROPIC_OAUTH_SCOPE,
    });

    let resp: Value = client
        .post("https://platform.claude.com/v1/oauth/token")
        .header("Content-Type", "application/json")
        .json(&params)
        .send()
        .map_err(|error| {
            clear_anthropic_oauth_tokens();
            anyhow!(error)
        })?
        .error_for_status()
        .map_err(|error| {
            clear_anthropic_oauth_tokens();
            anyhow!(error)
        })?
        .json()
        .map_err(|error| {
            clear_anthropic_oauth_tokens();
            anyhow!(error)
        })?;

    let new_access = resp["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("No access_token in Anthropic refresh response"))?
        .to_string();
    let expires_at = token_expiry_from_now(resp["expires_in"].as_i64());
    let new_refresh = resp["refresh_token"]
        .as_str()
        .map(str::to_string)
        .unwrap_or(refresh_token);
    let mut stored = load_stored_anthropic_secret();
    stored.refresh_token = new_refresh.clone();
    if let Some(subscription_type) = pick_str(
        &resp,
        &[
            "subscriptionType",
            "subscription_type",
            "planType",
            "plan_type",
        ],
    ) {
        stored.subscription_type = subscription_type.to_string();
    }
    if let Some(rate_limit_tier) = pick_str(&resp, &["rateLimitTier", "rate_limit_tier", "tier"]) {
        stored.rate_limit_tier = rate_limit_tier.to_string();
    }
    update_in_memory_anthropic_session(
        Some(new_access.clone()),
        Some(expires_at),
        Some(new_refresh),
        is_non_empty(&stored.subscription_type).then_some(stored.subscription_type.clone()),
        is_non_empty(&stored.rate_limit_tier).then_some(stored.rate_limit_tier.clone()),
    );
    if let Err(error) = persist_anthropic_secret(&stored) {
        clear_in_memory_anthropic_session();
        return Err(error);
    }

    Ok(new_access)
}

#[derive(Debug, Clone)]
struct OpenAiOAuthUsageData {
    account_id: String,
    plan_type: String,
    primary_percent: f64,
    secondary_percent: f64,
    primary_reset_at: Option<String>,
    secondary_reset_at: Option<String>,
}

#[derive(Debug, Clone)]
struct AnthropicOAuthUsageData {
    subscription_type: String,
    rate_limit_tier: String,
    session_used: f64,
    week_used: f64,
    session_reset_at: Option<String>,
    week_reset_at: Option<String>,
}

fn openai_oauth_plan_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown") {
        return "Subscription".to_string();
    }

    let mut chars = trimmed.chars();
    match chars.next() {
        None => "Subscription".to_string(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn anthropic_oauth_plan_label(value: Option<String>) -> String {
    value.unwrap_or_else(|| "Subscription".to_string())
}

fn fetch_openai_oauth_usage_value(
    access_token: &str,
    account_id: &str,
) -> std::result::Result<Value, ApiFetchError> {
    let client = client_with_timeout().map_err(ApiFetchError::Transport)?;
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("User-Agent", "opencode/0.1");

    if !account_id.is_empty() {
        req = req.header("ChatGPT-Account-Id", account_id);
    }

    let resp = req
        .send()
        .map_err(|error| ApiFetchError::Transport(error.into()))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(ApiFetchError::Http { status, body });
    }

    resp.json()
        .map_err(|error| ApiFetchError::InvalidResponse(error.into()))
}

fn fetch_anthropic_oauth_usage_value(
    access_token: &str,
) -> std::result::Result<Value, ApiFetchError> {
    let client = client_with_timeout().map_err(ApiFetchError::Transport)?;
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("anthropic-beta", ANTHROPIC_OAUTH_BETA_HEADER)
        .header("User-Agent", "usageguard/0.1")
        .send()
        .map_err(|error| ApiFetchError::Transport(error.into()))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(ApiFetchError::Http { status, body });
    }

    resp.json()
        .map_err(|error| ApiFetchError::InvalidResponse(error.into()))
}

fn parse_openai_oauth_usage_data(value: &Value) -> Result<OpenAiOAuthUsageData> {
    let account_id = value["account_id"]
        .as_str()
        .or_else(|| value["user_id"].as_str())
        .unwrap_or_default()
        .to_string();
    let primary_percent = value
        .pointer("/rate_limit/primary_window/used_percent")
        .or_else(|| value.pointer("/primary_window/used_percent"))
        .and_then(|entry| entry.as_f64());
    let secondary_percent = value
        .pointer("/rate_limit/secondary_window/used_percent")
        .or_else(|| value.pointer("/secondary_window/used_percent"))
        .and_then(|entry| entry.as_f64());

    if primary_percent.is_none() && secondary_percent.is_none() {
        return Err(anyhow!(
            "OpenAI oauth usage response missing supported quota window data"
        ));
    }

    let primary_reset_at = openai_oauth_window_reset_at(
        value,
        "primary_window",
        "primaryWindow",
        "short_window",
    );
    let secondary_reset_at = openai_oauth_window_reset_at(
        value,
        "secondary_window",
        "secondaryWindow",
        "long_window",
    );

    Ok(OpenAiOAuthUsageData {
        account_id,
        plan_type: openai_oauth_plan_label(value["plan_type"].as_str().unwrap_or_default()),
        primary_percent: primary_percent
            .or(secondary_percent)
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        secondary_percent: secondary_percent
            .or(primary_percent)
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        primary_reset_at: primary_reset_at
            .clone()
            .or_else(|| secondary_reset_at.clone()),
        secondary_reset_at: secondary_reset_at.or(primary_reset_at),
    })
}

fn do_fetch_wham_usage(access_token: &str, account_id: &str) -> Result<UsageSnapshot> {
    let value = fetch_openai_oauth_usage_value(access_token, account_id).map_err(|error| {
        let detail = match &error {
            ApiFetchError::Http { status, .. } => format!("HTTP {status}"),
            ApiFetchError::Transport(error) | ApiFetchError::InvalidResponse(error) => {
                error.to_string()
            }
        };
        anyhow!(detail)
    })?;
    parse_wham_usage_response(&value)
}

fn do_fetch_anthropic_oauth_usage(access_token: &str) -> Result<UsageSnapshot> {
    let value = fetch_anthropic_oauth_usage_value(access_token).map_err(|error| {
        let detail = match &error {
            ApiFetchError::Http { status, .. } => format!("HTTP {status}"),
            ApiFetchError::Transport(error) | ApiFetchError::InvalidResponse(error) => {
                error.to_string()
            }
        };
        anyhow!(detail)
    })?;
    parse_anthropic_oauth_usage_response(&value)
}

fn parse_wham_usage_response(value: &Value) -> Result<UsageSnapshot> {
    let usage = parse_openai_oauth_usage_data(value)?;
    let plan_type = usage.plan_type.clone();

    let mut stored = load_stored_openai_secret();
    if is_non_empty(&usage.account_id) {
        stored.account_id = usage.account_id.clone();
    }
    if is_non_empty(&usage.plan_type) {
        stored.plan_type = usage.plan_type.clone();
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
    let primary_percent = usage.primary_percent;
    let secondary_percent = usage.secondary_percent;

    // Capitalise first letter: "pro" → "Pro"
    let plan_display = plan_type;

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
        api_metrics: None,
        primary_reset_at: usage.primary_reset_at.clone(),
        secondary_reset_at: usage.secondary_reset_at.clone(),
    })
}

fn anthropic_oauth_bucket_value(bucket: &Value) -> Option<f64> {
    match bucket {
        Value::Number(number) => number.as_f64().map(|value| value.clamp(0.0, 100.0)),
        Value::Object(_) => pick_f64(bucket, &["utilization", "usage", "percent", "value"])
            .map(|value| value.clamp(0.0, 100.0)),
        _ => None,
    }
}

fn openai_oauth_window_reset_at(
    value: &Value,
    primary_key: &str,
    camel_key: &str,
    fallback_key: &str,
) -> Option<String> {
    [
        format!("/rate_limit/{primary_key}/resets_at"),
        format!("/rate_limit/{primary_key}/reset_at"),
        format!("/rate_limit/{primary_key}/resetsAt"),
        format!("/rate_limit/{primary_key}/resetAt"),
        format!("/rate_limit/{camel_key}/resets_at"),
        format!("/rate_limit/{camel_key}/reset_at"),
        format!("/rate_limit/{camel_key}/resetsAt"),
        format!("/rate_limit/{camel_key}/resetAt"),
        format!("/{primary_key}/resets_at"),
        format!("/{primary_key}/reset_at"),
        format!("/{primary_key}/resetsAt"),
        format!("/{primary_key}/resetAt"),
        format!("/{camel_key}/resets_at"),
        format!("/{camel_key}/reset_at"),
        format!("/{camel_key}/resetsAt"),
        format!("/{camel_key}/resetAt"),
        format!("/{fallback_key}/resets_at"),
        format!("/{fallback_key}/reset_at"),
        format!("/{fallback_key}/resetsAt"),
        format!("/{fallback_key}/resetAt"),
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
    .map(str::to_string)
}

fn anthropic_oauth_bucket<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn anthropic_oauth_bucket_percent(value: &Value, keys: &[&str]) -> Option<f64> {
    anthropic_oauth_bucket(value, keys)
        .and_then(anthropic_oauth_bucket_value)
        .or_else(|| {
            keys.iter().find_map(|key| {
                let utilization_key = format!("{key}_utilization");
                value
                    .get(utilization_key.as_str())
                    .and_then(anthropic_oauth_bucket_value)
            })
        })
}

fn anthropic_oauth_bucket_reset_at(value: &Value, keys: &[&str]) -> Option<String> {
    anthropic_oauth_bucket(value, keys)
        .and_then(|bucket| pick_str(bucket, &["resets_at", "reset_at", "resetsAt", "resetAt"]))
        .map(str::to_string)
}

fn parse_anthropic_oauth_usage_data(value: &Value) -> Result<AnthropicOAuthUsageData> {
    let five_hour_keys = &["five_hour", "fiveHour", "5_hour", "short_term", "shortTerm"];
    let seven_day_keys = &[
        "seven_day",
        "seven_day_all",
        "daily",
        "sevenDayAll",
        "7_day_all",
        "long_term",
        "longTerm",
        "weekly",
    ];
    let five_hour_percent = anthropic_oauth_bucket_percent(
        value,
        five_hour_keys,
    );
    let seven_day_percent = anthropic_oauth_bucket_percent(value, seven_day_keys);

    if five_hour_percent.is_none() && seven_day_percent.is_none() {
        return Err(anyhow!(
            "Anthropic oauth usage response missing supported utilization buckets"
        ));
    }

    let five_hour_reset_at = anthropic_oauth_bucket_reset_at(value, five_hour_keys);
    let seven_day_reset_at = anthropic_oauth_bucket_reset_at(value, seven_day_keys);

    Ok(AnthropicOAuthUsageData {
        subscription_type: pick_str(
            value,
            &[
                "subscriptionType",
                "subscription_type",
                "planType",
                "plan_type",
            ],
        )
        .unwrap_or_default()
        .to_string(),
        rate_limit_tier: pick_str(value, &["rateLimitTier", "rate_limit_tier", "tier"])
            .unwrap_or_default()
            .to_string(),
        session_used: five_hour_percent
            .or(seven_day_percent)
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        week_used: seven_day_percent
            .or(five_hour_percent)
            .unwrap_or(0.0)
            .clamp(0.0, 100.0),
        session_reset_at: five_hour_reset_at
            .clone()
            .or_else(|| seven_day_reset_at.clone()),
        week_reset_at: seven_day_reset_at.or(five_hour_reset_at),
    })
}

fn anthropic_oauth_snapshot_from_usage(
    usage: &AnthropicOAuthUsageData,
    plan_type: Option<String>,
) -> UsageSnapshot {
    UsageSnapshot {
        provider: "anthropic".into(),
        account_label: format!("Claude {}", anthropic_oauth_plan_label(plan_type)),
        spent_usd: usage.week_used,
        limit_usd: 100.0,
        tokens_in: usage.session_used.round() as u64,
        tokens_out: 0,
        inactive_hours: 0,
        source: "oauth".to_string(),
        status_code: None,
        status_message: None,
        api_metrics: None,
        primary_reset_at: usage.session_reset_at.clone(),
        secondary_reset_at: usage.week_reset_at.clone(),
    }
}

fn parse_anthropic_oauth_usage_response(value: &Value) -> Result<UsageSnapshot> {
    let usage = parse_anthropic_oauth_usage_data(value)?;
    let mut stored = load_stored_anthropic_secret();
    if is_non_empty(&usage.subscription_type) {
        stored.subscription_type = usage.subscription_type.clone();
    }
    if is_non_empty(&usage.rate_limit_tier) {
        stored.rate_limit_tier = usage.rate_limit_tier.clone();
    }
    let _ = persist_anthropic_secret(&stored);
    update_in_memory_anthropic_session(
        None,
        None,
        None,
        is_non_empty(&stored.subscription_type).then_some(stored.subscription_type.clone()),
        is_non_empty(&stored.rate_limit_tier).then_some(stored.rate_limit_tier.clone()),
    );

    Ok(anthropic_oauth_snapshot_from_usage(
        &usage,
        anthropic_plan_type_from_fields(&stored.subscription_type, &stored.rate_limit_tier),
    ))
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
    allow_env_fallback: bool,
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_config_dir()?.join("config.json"))
}

fn legacy_endpoint(cfg: &ApiCredentials, provider_id: &str) -> Option<String> {
    match provider_id {
        "openai" => cfg.openai_costs_endpoint.clone(),
        "anthropic" => cfg.anthropic_costs_endpoint.clone(),
        _ => None,
    }
}

fn clear_legacy_endpoint(cfg: &mut ApiCredentials, provider_id: &str) {
    match provider_id {
        "openai" => cfg.openai_costs_endpoint = None,
        "anthropic" => cfg.anthropic_costs_endpoint = None,
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

fn reject_legacy_individual_accounts(raw: &Value, path: &std::path::Path) -> Result<()> {
    let Some(accounts) = raw
        .get("provider_accounts")
        .and_then(|value| value.as_array())
    else {
        return Ok(());
    };

    for account in accounts {
        let access_mode = account
            .get("access_mode")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if access_mode != "individual" {
            continue;
        }

        let provider = account
            .get("provider")
            .and_then(|value| value.as_str())
            .unwrap_or("provider");
        let label = account
            .get("label")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unnamed account");
        anyhow::bail!(
            "Config contains unsupported individual API account '{label}' for {provider}. Remove it from {} and restart UsageGuard.",
            path.display()
        );
    }

    Ok(())
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read config file: {}", path.display()))?;
    let raw_value = serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("Invalid config JSON: {}", path.display()))?;
    reject_legacy_individual_accounts(&raw_value, &path)?;
    let mut cfg = serde_json::from_value::<AppConfig>(raw_value)
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

    for provider_id in ["openai", "anthropic"] {
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

    let normalized_refresh_interval = clamp_refresh_interval_secs(cfg.refresh_interval_secs);
    if cfg.refresh_interval_secs != normalized_refresh_interval {
        cfg.refresh_interval_secs = normalized_refresh_interval;
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
            allow_env_fallback: true,
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
        allow_env_fallback: false,
    })
}

pub fn provider_snapshots(cfg: &AppConfig) -> Vec<UsageSnapshot> {
    let mut items: Vec<UsageSnapshot> = vec![];

    // OAuth subscriptions first (consumer plans before API-key sources)
    if let Some(mut s) = fetch_openai_oauth_usage() {
        if let Some(label) = cfg
            .openai_oauth_label
            .as_deref()
            .filter(|l| !l.trim().is_empty())
        {
            s.account_label = label.to_string();
        }
        items.push(s);
    }
    if let Some(mut s) = fetch_anthropic_oauth_usage() {
        if let Some(label) = cfg
            .anthropic_oauth_label
            .as_deref()
            .filter(|l| !l.trim().is_empty())
        {
            s.account_label = label.to_string();
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationErrorKind {
    InvalidCredential,
    InsufficientAccess,
    UpstreamUnavailable,
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationError {
    kind: VerificationErrorKind,
    message: String,
}

impl VerificationError {
    fn new(kind: VerificationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> &VerificationErrorKind {
        &self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VerificationError {}

#[derive(Debug)]
enum ApiFetchError {
    Http {
        status: reqwest::StatusCode,
        body: String,
    },
    Transport(anyhow::Error),
    InvalidResponse(anyhow::Error),
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ApiWindowRollup {
    today: ApiMetricWindow,
    rolling_30d: ApiMetricWindow,
}

#[derive(Debug, Clone)]
pub struct OpenAiOAuthVerification {
    pub account_id: String,
    pub plan_type: String,
}

#[derive(Debug, Clone)]
pub struct AnthropicOAuthVerification {
    pub subscription_type: String,
    pub rate_limit_tier: String,
    pub plan_type: String,
}

fn verification_error_priority(kind: &VerificationErrorKind) -> u8 {
    match kind {
        VerificationErrorKind::InsufficientAccess => 0,
        VerificationErrorKind::InvalidCredential => 1,
        VerificationErrorKind::InvalidResponse => 2,
        VerificationErrorKind::UpstreamUnavailable => 3,
    }
}

fn preferred_verification_error(
    left: VerificationError,
    right: VerificationError,
) -> VerificationError {
    if verification_error_priority(left.kind()) <= verification_error_priority(right.kind()) {
        left
    } else {
        right
    }
}

fn validation_error_for_http_status(
    status: reqwest::StatusCode,
    invalid_message: impl Into<String>,
    forbidden_message: impl Into<String>,
    unavailable_message: impl Into<String>,
) -> VerificationError {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        VerificationError::new(VerificationErrorKind::InvalidCredential, invalid_message)
    } else if status == reqwest::StatusCode::FORBIDDEN {
        VerificationError::new(VerificationErrorKind::InsufficientAccess, forbidden_message)
    } else {
        VerificationError::new(
            VerificationErrorKind::UpstreamUnavailable,
            unavailable_message,
        )
    }
}

fn validation_error_from_api_fetch(
    error: &ApiFetchError,
    invalid_message: impl Into<String>,
    forbidden_message: impl Into<String>,
    unavailable_message: impl Into<String>,
    invalid_response_message: impl Into<String>,
) -> VerificationError {
    let unavailable_message = unavailable_message.into();
    match error {
        ApiFetchError::Http { status, .. } => validation_error_for_http_status(
            *status,
            invalid_message,
            forbidden_message,
            unavailable_message,
        ),
        ApiFetchError::Transport(_) => {
            VerificationError::new(VerificationErrorKind::UpstreamUnavailable, unavailable_message)
        }
        ApiFetchError::InvalidResponse(_) => VerificationError::new(
            VerificationErrorKind::InvalidResponse,
            invalid_response_message,
        ),
    }
}

fn utc_day_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now.date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight")
        .and_utc()
}

fn rolling_30d_window(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>) {
    let today_start = utc_day_start(now);
    let rolling_start = today_start - Duration::days(29);
    let next_day_start = today_start + Duration::days(1);
    (rolling_start, today_start, next_day_start)
}

fn apply_rollup_value(
    rollup: &mut ApiWindowRollup,
    bucket_start: DateTime<Utc>,
    today_start: DateTime<Utc>,
    spend_usd: f64,
    tokens_in: u64,
    tokens_out: u64,
    requests: Option<u64>,
) {
    rollup.rolling_30d.spend_usd += spend_usd;
    rollup.rolling_30d.tokens_in += tokens_in;
    rollup.rolling_30d.tokens_out += tokens_out;
    if let Some(count) = requests {
        let next = rollup.rolling_30d.requests.unwrap_or(0) + count;
        rollup.rolling_30d.requests = Some(next);
    }

    if bucket_start >= today_start {
        rollup.today.spend_usd += spend_usd;
        rollup.today.tokens_in += tokens_in;
        rollup.today.tokens_out += tokens_out;
        if let Some(count) = requests {
            let next = rollup.today.requests.unwrap_or(0) + count;
            rollup.today.requests = Some(next);
        }
    }
}

fn client_with_timeout() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(Into::into)
}

fn fetch_json_value(
    client: &reqwest::blocking::Client,
    url: &str,
    method: HttpMethod,
    auth: Option<(&str, AuthMode, &str)>,
    headers: &[(&str, String)],
    request_body: Option<&Value>,
    query: &[(&str, String)],
) -> std::result::Result<Value, ApiFetchError> {
    let mut req = match method {
        HttpMethod::Get => client.get(url),
    };
    if let Some((header, auth_mode, key)) = auth {
        req = apply_auth(req, header, auth_mode, key);
    }
    if !query.is_empty() {
        req = req.query(query);
    }
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    if let Some(body) = request_body {
        req = req.json(body);
    }

    let res = req
        .send()
        .map_err(|error| ApiFetchError::Transport(error.into()))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().unwrap_or_default();
        return Err(ApiFetchError::Http { status, body });
    }

    res.json()
        .map_err(|error| ApiFetchError::InvalidResponse(error.into()))
}

fn openai_cost_amount_usd(row: &Value) -> Option<f64> {
    row.get("amount")
        .and_then(|amount| amount.get("value"))
        .and_then(|value| value.as_f64())
        .or_else(|| pick_f64(row, &["cost_usd", "spent_usd", "amount_usd"]))
}

fn parse_openai_cost_rollup(value: &Value, today_start: DateTime<Utc>) -> Result<ApiWindowRollup> {
    let buckets = value
        .get("data")
        .and_then(|data| data.as_array())
        .context("OpenAI costs response missing data buckets")?;
    let mut rollup = ApiWindowRollup::default();

    for bucket in buckets {
        let bucket_start = bucket
            .get("start_time")
            .and_then(|entry| entry.as_i64())
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
            .context("OpenAI cost bucket missing start_time")?;
        let spend_usd = bucket
            .get("results")
            .and_then(|results| results.as_array())
            .map(|results| {
                results
                    .iter()
                    .filter_map(openai_cost_amount_usd)
                    .sum::<f64>()
            })
            .unwrap_or_else(|| openai_cost_amount_usd(bucket).unwrap_or(0.0));

        apply_rollup_value(
            &mut rollup,
            bucket_start,
            today_start,
            spend_usd,
            0,
            0,
            None,
        );
    }

    Ok(rollup)
}

fn openai_usage_row_requests(row: &Value) -> Option<u64> {
    pick_u64(
        row,
        &[
            "num_model_requests",
            "model_requests",
            "requests",
            "request_count",
        ],
    )
}

fn parse_openai_usage_rollup(value: &Value, today_start: DateTime<Utc>) -> Result<ApiWindowRollup> {
    let buckets = value
        .get("data")
        .and_then(|data| data.as_array())
        .context("OpenAI usage response missing data buckets")?;
    let mut rollup = ApiWindowRollup::default();

    for bucket in buckets {
        let bucket_start = bucket
            .get("start_time")
            .and_then(|entry| entry.as_i64())
            .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
            .context("OpenAI usage bucket missing start_time")?;

        let (tokens_in, tokens_out, requests) = bucket
            .get("results")
            .and_then(|results| results.as_array())
            .map(|results| {
                results.iter().fold((0_u64, 0_u64, None), |acc, row| {
                    let tokens_in = acc.0
                        + pick_u64(row, &["input_tokens", "tokens_in", "total_input_tokens"])
                            .unwrap_or(0);
                    let tokens_out = acc.1
                        + pick_u64(row, &["output_tokens", "tokens_out", "total_output_tokens"])
                            .unwrap_or(0);
                    let requests = match (acc.2, openai_usage_row_requests(row)) {
                        (Some(existing), Some(next)) => Some(existing + next),
                        (Some(existing), None) => Some(existing),
                        (None, Some(next)) => Some(next),
                        (None, None) => None,
                    };
                    (tokens_in, tokens_out, requests)
                })
            })
            .unwrap_or((
                pick_u64(bucket, &["input_tokens", "tokens_in", "total_input_tokens"]).unwrap_or(0),
                pick_u64(
                    bucket,
                    &["output_tokens", "tokens_out", "total_output_tokens"],
                )
                .unwrap_or(0),
                openai_usage_row_requests(bucket),
            ));

        apply_rollup_value(
            &mut rollup,
            bucket_start,
            today_start,
            0.0,
            tokens_in,
            tokens_out,
            requests,
        );
    }

    Ok(rollup)
}

fn parse_iso_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_anthropic_bucket_start(bucket: &Value) -> Option<DateTime<Utc>> {
    pick_str(bucket, &["starting_at", "start_time"])
        .and_then(parse_iso_datetime)
        .or_else(|| {
            bucket
                .get("start_time")
                .and_then(|value| value.as_i64())
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
        })
}

fn anthropic_amount_minor_to_usd(row: &Value) -> Option<f64> {
    row.get("amount")
        .and_then(|amount| amount.as_str())
        .and_then(|amount| amount.parse::<f64>().ok())
        .map(|minor| minor / 100.0)
        .or_else(|| pick_f64(row, &["cost_usd", "amount_usd", "spent_usd"]))
}

fn parse_anthropic_cost_rollup(
    value: &Value,
    today_start: DateTime<Utc>,
) -> Result<ApiWindowRollup> {
    let buckets = value
        .get("data")
        .and_then(|data| data.as_array())
        .context("Anthropic cost report missing data buckets")?;
    let mut rollup = ApiWindowRollup::default();

    for bucket in buckets {
        let bucket_start =
            parse_anthropic_bucket_start(bucket).context("Anthropic cost bucket missing start")?;
        let spend_usd = bucket
            .get("results")
            .and_then(|results| results.as_array())
            .map(|results| {
                results
                    .iter()
                    .filter_map(anthropic_amount_minor_to_usd)
                    .sum::<f64>()
            })
            .unwrap_or_else(|| anthropic_amount_minor_to_usd(bucket).unwrap_or(0.0));

        apply_rollup_value(
            &mut rollup,
            bucket_start,
            today_start,
            spend_usd,
            0,
            0,
            None,
        );
    }

    Ok(rollup)
}

fn anthropic_usage_input_tokens(row: &Value) -> u64 {
    pick_u64(row, &["input_tokens", "tokens_in", "total_input_tokens"]).unwrap_or(0)
        + pick_u64(row, &["uncached_input_tokens"]).unwrap_or(0)
        + pick_u64(row, &["cache_read_input_tokens"]).unwrap_or(0)
        + row
            .pointer("/cache_creation_input_tokens/ephemeral_1h_input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
        + row
            .pointer("/cache_creation_input_tokens/ephemeral_5m_input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
        + pick_u64(
            row,
            &[
                "cache_creation_input_tokens",
                "cache_creation_ephemeral_1h_input_tokens",
                "cache_creation_ephemeral_5m_input_tokens",
            ],
        )
        .unwrap_or(0)
}

fn parse_anthropic_usage_rollup(
    value: &Value,
    today_start: DateTime<Utc>,
) -> Result<ApiWindowRollup> {
    let buckets = value
        .get("data")
        .and_then(|data| data.as_array())
        .context("Anthropic usage report missing data buckets")?;
    let mut rollup = ApiWindowRollup::default();

    for bucket in buckets {
        let bucket_start =
            parse_anthropic_bucket_start(bucket).context("Anthropic usage bucket missing start")?;

        let (tokens_in, tokens_out) = bucket
            .get("results")
            .and_then(|results| results.as_array())
            .map(|results| {
                results.iter().fold((0_u64, 0_u64), |acc, row| {
                    let tokens_in = acc.0 + anthropic_usage_input_tokens(row);
                    let tokens_out = acc.1
                        + pick_u64(row, &["output_tokens", "tokens_out", "total_output_tokens"])
                            .unwrap_or(0);
                    (tokens_in, tokens_out)
                })
            })
            .unwrap_or((
                anthropic_usage_input_tokens(bucket),
                pick_u64(
                    bucket,
                    &["output_tokens", "tokens_out", "total_output_tokens"],
                )
                .unwrap_or(0),
            ));

        apply_rollup_value(
            &mut rollup,
            bucket_start,
            today_start,
            0.0,
            tokens_in,
            tokens_out,
            None,
        );
    }

    Ok(rollup)
}

fn build_api_metric_snapshot(
    provider: &str,
    label: &str,
    source: &str,
    metrics: ApiMetricCard,
    inactive_hours: u32,
    status_code: Option<&str>,
    status_message: Option<String>,
) -> UsageSnapshot {
    UsageSnapshot {
        provider: provider.to_string(),
        account_label: label.to_string(),
        spent_usd: metrics.rolling_30d.spend_usd,
        limit_usd: 0.0,
        tokens_in: metrics.rolling_30d.tokens_in,
        tokens_out: metrics.rolling_30d.tokens_out,
        inactive_hours,
        source: source.to_string(),
        status_code: status_code.map(str::to_string),
        status_message,
        api_metrics: Some(metrics),
        primary_reset_at: None,
        secondary_reset_at: None,
    }
}

fn openai_admin_status_from_error(error: &ApiFetchError) -> (&'static str, String) {
    match error {
        ApiFetchError::Http { status, .. } if *status == reqwest::StatusCode::UNAUTHORIZED => (
            "admin_api_key_required",
            "OpenAI Admin API key or equivalent org usage permission required.".to_string(),
        ),
        ApiFetchError::Http { status, .. } if *status == reqwest::StatusCode::FORBIDDEN => (
            "admin_api_access_denied",
            "OpenAI Admin API key lacks organization usage access.".to_string(),
        ),
        ApiFetchError::InvalidResponse(_) => (
            "api_invalid_response",
            "OpenAI usage endpoint returned unusable data.".to_string(),
        ),
        _ => (
            "api_usage_unavailable",
            "Unable to load OpenAI API usage right now.".to_string(),
        ),
    }
}

fn anthropic_admin_status_from_error(error: &ApiFetchError) -> (&'static str, String) {
    match error {
        ApiFetchError::Http { status, .. } if *status == reqwest::StatusCode::UNAUTHORIZED => (
            "admin_api_key_required",
            "Anthropic Admin API key required for organization usage.".to_string(),
        ),
        ApiFetchError::Http { status, .. } if *status == reqwest::StatusCode::FORBIDDEN => (
            "admin_api_access_denied",
            "Anthropic Admin API key lacks organization usage access.".to_string(),
        ),
        ApiFetchError::InvalidResponse(_) => (
            "api_invalid_response",
            "Anthropic usage endpoint returned unusable data.".to_string(),
        ),
        _ => (
            "api_usage_unavailable",
            "Unable to load Anthropic API usage right now.".to_string(),
        ),
    }
}

fn push_partial_status(target: &mut Vec<String>, prefix: &str, error: &ApiFetchError) {
    let detail = match error {
        ApiFetchError::Http { status, body } => {
            if body.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                format!("HTTP {status}")
            }
        }
        ApiFetchError::Transport(error) | ApiFetchError::InvalidResponse(error) => {
            error.to_string()
        }
    };
    target.push(format!("{prefix}: {detail}"));
}

fn strict_openai_api_validation_error(error: &ApiFetchError) -> VerificationError {
    validation_error_from_api_fetch(
        error,
        "OpenAI API key is invalid. Nothing was saved.",
        "OpenAI API key does not have organization usage access. Nothing was saved.",
        "OpenAI verification could not reach the usage service right now. Nothing was saved.",
        "OpenAI verification returned unusable usage data. Nothing was saved.",
    )
}

fn strict_anthropic_api_validation_error(error: &ApiFetchError) -> VerificationError {
    validation_error_from_api_fetch(
        error,
        "Anthropic API key is invalid. Nothing was saved.",
        "Anthropic API key does not have organization usage access. Nothing was saved.",
        "Anthropic verification could not reach the usage service right now. Nothing was saved.",
        "Anthropic verification returned unusable usage data. Nothing was saved.",
    )
}

fn strict_openai_oauth_validation_error(error: &ApiFetchError) -> VerificationError {
    validation_error_from_api_fetch(
        error,
        "ChatGPT sign-in could not be verified. Sign in again. Nothing was saved.",
        "ChatGPT subscription usage access was denied. Nothing was saved.",
        "ChatGPT verification could not reach the usage service right now. Nothing was saved.",
        "ChatGPT verification returned unusable subscription data. Nothing was saved.",
    )
}

fn strict_anthropic_oauth_validation_error(error: &ApiFetchError) -> VerificationError {
    validation_error_from_api_fetch(
        error,
        "Claude sign-in could not be verified. Sign in again. Nothing was saved.",
        "Claude subscription usage access was denied. Nothing was saved.",
        "Claude verification could not reach the usage service right now. Nothing was saved.",
        "Claude verification returned unusable subscription data. Nothing was saved.",
    )
}

fn verify_openai_organization_api_key(api_key: &str) -> std::result::Result<(), VerificationError> {
    let client = client_with_timeout().map_err(|error| {
        VerificationError::new(
            VerificationErrorKind::UpstreamUnavailable,
            format!("OpenAI verification could not start: {error}. Nothing was saved."),
        )
    })?;
    let (rolling_start, today_start, _) = rolling_30d_window(Utc::now());
    let query = vec![
        ("start_time", rolling_start.timestamp().to_string()),
        ("bucket_width", "1d".to_string()),
        ("limit", "30".to_string()),
    ];

    let cost_result = fetch_json_value(
        &client,
        "https://api.openai.com/v1/organization/costs",
        HttpMethod::Get,
        Some(("Authorization", AuthMode::Bearer, api_key)),
        &[],
        None,
        &query,
    )
    .and_then(|value| {
        parse_openai_cost_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    })
    .map_err(|error| strict_openai_api_validation_error(&error));

    let usage_result = fetch_json_value(
        &client,
        "https://api.openai.com/v1/organization/usage/completions",
        HttpMethod::Get,
        Some(("Authorization", AuthMode::Bearer, api_key)),
        &[],
        None,
        &query,
    )
    .and_then(|value| {
        parse_openai_usage_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    })
    .map_err(|error| strict_openai_api_validation_error(&error));

    strict_api_rollups(cost_result, usage_result).map(|_| ())
}

fn verify_anthropic_organization_api_key(
    api_key: &str,
) -> std::result::Result<(), VerificationError> {
    if !api_key.trim().starts_with("sk-ant-admin") {
        return Err(VerificationError::new(
            VerificationErrorKind::InvalidCredential,
            "Anthropic Admin API key required for organization usage. Nothing was saved.",
        ));
    }

    let client = client_with_timeout().map_err(|error| {
        VerificationError::new(
            VerificationErrorKind::UpstreamUnavailable,
            format!("Anthropic verification could not start: {error}. Nothing was saved."),
        )
    })?;
    let (rolling_start, today_start, next_day_start) = rolling_30d_window(Utc::now());
    let query = vec![
        ("starting_at", rolling_start.to_rfc3339()),
        ("ending_at", next_day_start.to_rfc3339()),
        ("granularity", "1d".to_string()),
    ];
    let headers = vec![("anthropic-version", "2023-06-01".to_string())];

    let usage_result = fetch_json_value(
        &client,
        "https://api.anthropic.com/v1/organizations/usage_report/messages",
        HttpMethod::Get,
        Some(("x-api-key", AuthMode::Raw, api_key)),
        &headers,
        None,
        &query,
    )
    .and_then(|value| {
        parse_anthropic_usage_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    })
    .map_err(|error| strict_anthropic_api_validation_error(&error));

    let cost_result = fetch_json_value(
        &client,
        "https://api.anthropic.com/v1/organizations/cost_report",
        HttpMethod::Get,
        Some(("x-api-key", AuthMode::Raw, api_key)),
        &headers,
        None,
        &query,
    )
    .and_then(|value| {
        parse_anthropic_cost_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    })
    .map_err(|error| strict_anthropic_api_validation_error(&error));

    strict_api_rollups(cost_result, usage_result).map(|_| ())
}

fn strict_api_rollups(
    cost_result: std::result::Result<ApiWindowRollup, VerificationError>,
    usage_result: std::result::Result<ApiWindowRollup, VerificationError>,
) -> std::result::Result<(ApiWindowRollup, ApiWindowRollup), VerificationError> {
    match (cost_result, usage_result) {
        (Ok(cost), Ok(usage)) => Ok((cost, usage)),
        (Err(left), Ok(_)) => Err(left),
        (Ok(_), Err(right)) => Err(right),
        (Err(left), Err(right)) => Err(preferred_verification_error(left, right)),
    }
}

pub fn verify_provider_api_key(
    provider_id: &str,
    api_key: &str,
) -> std::result::Result<(), VerificationError> {
    match provider_id {
        "openai" => verify_openai_organization_api_key(api_key),
        "anthropic" => verify_anthropic_organization_api_key(api_key),
        _ => Err(VerificationError::new(
            VerificationErrorKind::InvalidResponse,
            format!("Unsupported provider '{provider_id}'. Nothing was saved."),
        )),
    }
}

pub fn verify_openai_oauth_access_token(
    access_token: &str,
    account_id_hint: &str,
) -> std::result::Result<OpenAiOAuthVerification, VerificationError> {
    let value = fetch_openai_oauth_usage_value(access_token, account_id_hint)
        .map_err(|error| strict_openai_oauth_validation_error(&error))?;
    let usage = parse_openai_oauth_usage_data(&value).map_err(|_| {
        VerificationError::new(
            VerificationErrorKind::InvalidResponse,
            "ChatGPT verification returned unusable subscription data. Nothing was saved.",
        )
    })?;

    Ok(OpenAiOAuthVerification {
        account_id: usage.account_id,
        plan_type: usage.plan_type,
    })
}

pub fn verify_anthropic_oauth_access_token(
    access_token: &str,
    subscription_type_hint: &str,
    rate_limit_tier_hint: &str,
) -> std::result::Result<AnthropicOAuthVerification, VerificationError> {
    let value = fetch_anthropic_oauth_usage_value(access_token)
        .map_err(|error| strict_anthropic_oauth_validation_error(&error))?;
    let usage = parse_anthropic_oauth_usage_data(&value).map_err(|_| {
        VerificationError::new(
            VerificationErrorKind::InvalidResponse,
            "Claude verification returned unusable subscription data. Nothing was saved.",
        )
    })?;

    let mut subscription_type = usage.subscription_type;
    let mut rate_limit_tier = usage.rate_limit_tier;
    if !is_non_empty(&subscription_type) && is_non_empty(subscription_type_hint) {
        subscription_type = subscription_type_hint.to_string();
    }
    if !is_non_empty(&rate_limit_tier) && is_non_empty(rate_limit_tier_hint) {
        rate_limit_tier = rate_limit_tier_hint.to_string();
    }
    if !is_non_empty(&subscription_type) || !is_non_empty(&rate_limit_tier) {
        if let Some((local_subscription_type, local_rate_limit_tier)) =
            load_local_claude_oauth_metadata()
        {
            if !is_non_empty(&subscription_type) {
                subscription_type = local_subscription_type;
            }
            if !is_non_empty(&rate_limit_tier) {
                rate_limit_tier = local_rate_limit_tier;
            }
        }
    }

    Ok(AnthropicOAuthVerification {
        plan_type: anthropic_oauth_plan_label(anthropic_plan_type_from_fields(
            &subscription_type,
            &rate_limit_tier,
        )),
        subscription_type,
        rate_limit_tier,
    })
}

fn fetch_openai_api_snapshot(label: &str, api_key: &str) -> Result<UsageSnapshot> {
    let client = client_with_timeout()?;
    let (rolling_start, today_start, _) = rolling_30d_window(Utc::now());
    let query = vec![
        ("start_time", rolling_start.timestamp().to_string()),
        ("bucket_width", "1d".to_string()),
        ("limit", "30".to_string()),
    ];

    let cost_result = fetch_json_value(
        &client,
        "https://api.openai.com/v1/organization/costs",
        HttpMethod::Get,
        Some(("Authorization", AuthMode::Bearer, api_key)),
        &[],
        None,
        &query,
    )
    .and_then(|value| {
        parse_openai_cost_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    });

    let usage_result = fetch_json_value(
        &client,
        "https://api.openai.com/v1/organization/usage/completions",
        HttpMethod::Get,
        Some(("Authorization", AuthMode::Bearer, api_key)),
        &[],
        None,
        &query,
    )
    .and_then(|value| {
        parse_openai_usage_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    });

    match (&cost_result, &usage_result) {
        (Err(error), Err(_)) => {
            let (status_code, status_message) = openai_admin_status_from_error(error);
            return Ok(error_snapshot(
                "openai",
                label,
                "api",
                Some(status_code),
                Some(&status_message),
            ));
        }
        _ => {}
    }

    let mut metrics = ApiMetricCard::default();
    let mut status_parts = Vec::new();

    match cost_result {
        Ok(rollup) => {
            metrics.today.spend_usd = rollup.today.spend_usd;
            metrics.rolling_30d.spend_usd = rollup.rolling_30d.spend_usd;
        }
        Err(error) => push_partial_status(&mut status_parts, "Cost data unavailable", &error),
    }

    match usage_result {
        Ok(rollup) => {
            metrics.today.tokens_in = rollup.today.tokens_in;
            metrics.today.tokens_out = rollup.today.tokens_out;
            metrics.today.requests = rollup.today.requests;
            metrics.rolling_30d.tokens_in = rollup.rolling_30d.tokens_in;
            metrics.rolling_30d.tokens_out = rollup.rolling_30d.tokens_out;
            metrics.rolling_30d.requests = rollup.rolling_30d.requests;
        }
        Err(error) => {
            push_partial_status(&mut status_parts, "Completions usage unavailable", &error)
        }
    }

    Ok(build_api_metric_snapshot(
        "openai",
        label,
        "api",
        metrics,
        0,
        (!status_parts.is_empty()).then_some("api_partial_data"),
        (!status_parts.is_empty()).then_some(status_parts.join(" ")),
    ))
}

fn fetch_anthropic_api_snapshot(label: &str, api_key: &str) -> Result<UsageSnapshot> {
    if !api_key.trim().starts_with("sk-ant-admin") {
        return Ok(error_snapshot(
            "anthropic",
            label,
            "api",
            Some("admin_api_key_required"),
            Some("Anthropic Admin API key required for organization usage."),
        ));
    }

    let client = client_with_timeout()?;
    let (rolling_start, today_start, next_day_start) = rolling_30d_window(Utc::now());
    let query = vec![
        ("starting_at", rolling_start.to_rfc3339()),
        ("ending_at", next_day_start.to_rfc3339()),
        ("granularity", "1d".to_string()),
    ];
    let headers = vec![("anthropic-version", "2023-06-01".to_string())];

    let usage_result = fetch_json_value(
        &client,
        "https://api.anthropic.com/v1/organizations/usage_report/messages",
        HttpMethod::Get,
        Some(("x-api-key", AuthMode::Raw, api_key)),
        &headers,
        None,
        &query,
    )
    .and_then(|value| {
        parse_anthropic_usage_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    });

    let cost_result = fetch_json_value(
        &client,
        "https://api.anthropic.com/v1/organizations/cost_report",
        HttpMethod::Get,
        Some(("x-api-key", AuthMode::Raw, api_key)),
        &headers,
        None,
        &query,
    )
    .and_then(|value| {
        parse_anthropic_cost_rollup(&value, today_start).map_err(ApiFetchError::InvalidResponse)
    });

    match (&cost_result, &usage_result) {
        (Err(error), Err(_)) => {
            let (status_code, status_message) = anthropic_admin_status_from_error(error);
            return Ok(error_snapshot(
                "anthropic",
                label,
                "api",
                Some(status_code),
                Some(&status_message),
            ));
        }
        _ => {}
    }

    let mut metrics = ApiMetricCard::default();
    let mut status_parts = Vec::new();

    match cost_result {
        Ok(rollup) => {
            metrics.today.spend_usd = rollup.today.spend_usd;
            metrics.rolling_30d.spend_usd = rollup.rolling_30d.spend_usd;
        }
        Err(error) => push_partial_status(&mut status_parts, "Cost report unavailable", &error),
    }

    match usage_result {
        Ok(rollup) => {
            metrics.today.tokens_in = rollup.today.tokens_in;
            metrics.today.tokens_out = rollup.today.tokens_out;
            metrics.rolling_30d.tokens_in = rollup.rolling_30d.tokens_in;
            metrics.rolling_30d.tokens_out = rollup.rolling_30d.tokens_out;
        }
        Err(error) => push_partial_status(&mut status_parts, "Messages usage unavailable", &error),
    }

    Ok(build_api_metric_snapshot(
        "anthropic",
        label,
        "api",
        metrics,
        0,
        (!status_parts.is_empty()).then_some("api_partial_data"),
        (!status_parts.is_empty()).then_some(status_parts.join(" ")),
    ))
}

fn fetch_provider_snapshot(spec: ProviderSpec<'_>) -> Option<UsageSnapshot> {
    if let Some(log_env) = spec.usage_log_env {
        if let Ok(path) = std::env::var(log_env) {
            if let Ok(s) = snapshot_from_ndjson(&path, spec.id, spec.label) {
                return Some(s);
            }
        }
    }

    if let Some(key) = spec.api_key {
        let result = match spec.id {
            "openai" => fetch_openai_api_snapshot(spec.label, &key),
            "anthropic" => fetch_anthropic_api_snapshot(spec.label, &key),
            _ => {
                let endpoint = spec
                    .endpoint
                    .or_else(|| spec.default_endpoint.map(|v| v.to_string()));
                match endpoint {
                    Some(url) => snapshot_from_http_json(
                        &url,
                        spec.method,
                        Some((spec.auth_header, spec.auth_mode, key.as_str())),
                        &spec.extra_headers,
                        spec.request_body.as_ref(),
                        spec.id,
                        spec.label,
                        "api",
                    ),
                    None => Err(anyhow!("No endpoint configured")),
                }
            }
        };

        match result {
            Ok(snapshot) => return Some(snapshot),
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

    if spec.allow_env_fallback {
        env_fallback_snapshot(spec.id, spec.label, spec.env_prefix)
    } else {
        Some(error_snapshot(
            spec.id,
            spec.label,
            "api",
            Some("api_key_missing"),
            Some("API key missing for configured account."),
        ))
    }
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
        api_metrics: None,
        primary_reset_at: None,
        secondary_reset_at: None,
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
        api_metrics: None,
        primary_reset_at: None,
        secondary_reset_at: None,
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
    let api_metrics = value
        .get("api_metrics")
        .cloned()
        .and_then(|entry| serde_json::from_value::<ApiMetricCard>(entry).ok());

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
        api_metrics,
        primary_reset_at: None,
        secondary_reset_at: None,
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
        api_metrics: None,
        primary_reset_at: None,
        secondary_reset_at: None,
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
        api_metrics: None,
        primary_reset_at: None,
        secondary_reset_at: None,
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
            api_metrics: None,
            primary_reset_at: None,
            secondary_reset_at: None,
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
            api_metrics: None,
            primary_reset_at: None,
            secondary_reset_at: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_claude_credentials_override(name: &str, body: &str, test: impl FnOnce()) {
        let _guard = crate::secret_store::test_env_lock().lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "usageguard_core_claude_plan_{name}_{}",
            std::process::id()
        ));
        let config_root = root.join("config");
        let credentials_path = root.join(".claude").join(".credentials.json");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&config_root).unwrap();
        fs::create_dir_all(credentials_path.parent().unwrap()).unwrap();
        fs::write(&credentials_path, body).unwrap();

        std::env::set_var("USAGEGUARD_CONFIG_DIR_OVERRIDE", &config_root);
        std::env::set_var(CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV, &credentials_path);
        clear_in_memory_anthropic_session();
        let _ = SecretStore::clear();

        test();

        clear_in_memory_anthropic_session();
        let _ = SecretStore::clear();
        std::env::remove_var("USAGEGUARD_CONFIG_DIR_OVERRIDE");
        std::env::remove_var(CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV);
        let _ = fs::remove_dir_all(&root);
    }

    fn with_test_config_dir(name: &str, test: impl FnOnce()) {
        let _guard = crate::secret_store::test_env_lock().lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "usageguard_core_validation_{name}_{}",
            std::process::id()
        ));
        let config_root = root.join("config");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&config_root).unwrap();
        std::env::set_var("USAGEGUARD_CONFIG_DIR_OVERRIDE", &config_root);
        clear_in_memory_oauth_session();
        clear_in_memory_anthropic_session();
        let _ = SecretStore::clear();

        test();

        clear_in_memory_oauth_session();
        clear_in_memory_anthropic_session();
        let _ = SecretStore::clear();
        std::env::remove_var("USAGEGUARD_CONFIG_DIR_OVERRIDE");
        let _ = fs::remove_dir_all(&root);
    }

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
            api_metrics: None,
            primary_reset_at: None,
            secondary_reset_at: None,
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
        assert!(snap.api_metrics.is_none());
    }

    #[test]
    fn parse_openai_cost_rollup_aggregates_today_and_30d() {
        let today_start = chrono::NaiveDate::from_ymd_opt(2026, 3, 9)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let yesterday_start = today_start - Duration::days(1);
        let value: Value = serde_json::json!({
            "data": [
                {
                    "start_time": today_start.timestamp(),
                    "results": [
                        { "amount": { "value": 1.25 } },
                        { "amount": { "value": 0.75 } }
                    ]
                },
                {
                    "start_time": yesterday_start.timestamp(),
                    "results": [
                        { "amount": { "value": 2.50 } }
                    ]
                }
            ]
        });

        let rollup = parse_openai_cost_rollup(&value, today_start).unwrap();
        assert!((rollup.today.spend_usd - 2.0).abs() < f64::EPSILON);
        assert!((rollup.rolling_30d.spend_usd - 4.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_openai_usage_rollup_aggregates_tokens_and_requests() {
        let today_start = chrono::NaiveDate::from_ymd_opt(2026, 3, 9)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let previous_start = today_start - Duration::days(2);
        let value: Value = serde_json::json!({
            "data": [
                {
                    "start_time": today_start.timestamp(),
                    "results": [
                        { "input_tokens": 150, "output_tokens": 60, "num_model_requests": 3 },
                        { "input_tokens": 50, "output_tokens": 15, "num_model_requests": 1 }
                    ]
                },
                {
                    "start_time": previous_start.timestamp(),
                    "results": [
                        { "input_tokens": 400, "output_tokens": 80, "num_model_requests": 5 }
                    ]
                }
            ]
        });

        let rollup = parse_openai_usage_rollup(&value, today_start).unwrap();
        assert_eq!(rollup.today.tokens_in, 200);
        assert_eq!(rollup.today.tokens_out, 75);
        assert_eq!(rollup.today.requests, Some(4));
        assert_eq!(rollup.rolling_30d.tokens_in, 600);
        assert_eq!(rollup.rolling_30d.tokens_out, 155);
        assert_eq!(rollup.rolling_30d.requests, Some(9));
    }

    #[test]
    fn parse_anthropic_usage_rollup_aggregates_message_tokens() {
        let today_start = chrono::NaiveDate::from_ymd_opt(2026, 3, 9)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let previous_start = today_start - Duration::days(1);
        let value: Value = serde_json::json!({
            "data": [
                {
                    "starting_at": today_start.to_rfc3339(),
                    "results": [
                        {
                            "uncached_input_tokens": 100,
                            "cache_read_input_tokens": 25,
                            "cache_creation_input_tokens": {
                                "ephemeral_1h_input_tokens": 10,
                                "ephemeral_5m_input_tokens": 5
                            },
                            "output_tokens": 30
                        }
                    ]
                },
                {
                    "starting_at": previous_start.to_rfc3339(),
                    "results": [
                        {
                            "uncached_input_tokens": 70,
                            "output_tokens": 12
                        }
                    ]
                }
            ]
        });

        let rollup = parse_anthropic_usage_rollup(&value, today_start).unwrap();
        assert_eq!(rollup.today.tokens_in, 140);
        assert_eq!(rollup.today.tokens_out, 30);
        assert_eq!(rollup.rolling_30d.tokens_in, 210);
        assert_eq!(rollup.rolling_30d.tokens_out, 42);
    }

    #[test]
    fn parse_anthropic_cost_rollup_converts_minor_units_to_usd() {
        let today_start = chrono::NaiveDate::from_ymd_opt(2026, 3, 9)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let previous_start = today_start - Duration::days(3);
        let value: Value = serde_json::json!({
            "data": [
                {
                    "starting_at": today_start.to_rfc3339(),
                    "results": [
                        { "amount": "12345" },
                        { "amount": "55" }
                    ]
                },
                {
                    "starting_at": previous_start.to_rfc3339(),
                    "results": [
                        { "amount": "200" }
                    ]
                }
            ]
        });

        let rollup = parse_anthropic_cost_rollup(&value, today_start).unwrap();
        assert!((rollup.today.spend_usd - 124.0).abs() < f64::EPSILON);
        assert!((rollup.rolling_30d.spend_usd - 126.0).abs() < f64::EPSILON);
    }

    #[test]
    fn admin_status_mapping_is_provider_specific() {
        let openai = ApiFetchError::Http {
            status: reqwest::StatusCode::FORBIDDEN,
            body: String::new(),
        };
        let anthropic = ApiFetchError::Http {
            status: reqwest::StatusCode::UNAUTHORIZED,
            body: String::new(),
        };

        let (openai_code, openai_message) = openai_admin_status_from_error(&openai);
        let (anthropic_code, anthropic_message) = anthropic_admin_status_from_error(&anthropic);

        assert_eq!(openai_code, "admin_api_access_denied");
        assert!(openai_message.contains("OpenAI Admin API key"));
        assert_eq!(anthropic_code, "admin_api_key_required");
        assert!(anthropic_message.contains("Anthropic Admin API key"));
    }

    #[test]
    fn strict_validation_classifies_invalid_credentials() {
        let error = strict_openai_api_validation_error(&ApiFetchError::Http {
            status: reqwest::StatusCode::UNAUTHORIZED,
            body: String::new(),
        });

        assert_eq!(error.kind(), &VerificationErrorKind::InvalidCredential);
        assert!(error.to_string().contains("invalid"));
    }

    #[test]
    fn strict_validation_classifies_upstream_outages() {
        let error = strict_openai_api_validation_error(&ApiFetchError::Http {
            status: reqwest::StatusCode::TOO_MANY_REQUESTS,
            body: String::new(),
        });

        assert_eq!(error.kind(), &VerificationErrorKind::UpstreamUnavailable);
        assert!(error.to_string().contains("Nothing was saved"));
    }

    #[test]
    fn strict_validation_classifies_invalid_provider_data() {
        let error = strict_anthropic_api_validation_error(&ApiFetchError::InvalidResponse(
            anyhow!("missing buckets"),
        ));

        assert_eq!(error.kind(), &VerificationErrorKind::InvalidResponse);
        assert!(error.to_string().contains("unusable"));
    }

    #[test]
    fn strict_rollup_validation_prefers_more_actionable_errors() {
        let result = strict_api_rollups(
            Err(strict_openai_api_validation_error(&ApiFetchError::Http {
                status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                body: String::new(),
            })),
            Err(strict_openai_api_validation_error(&ApiFetchError::Http {
                status: reqwest::StatusCode::UNAUTHORIZED,
                body: String::new(),
            })),
        );

        let error = result.unwrap_err();
        assert_eq!(error.kind(), &VerificationErrorKind::InvalidCredential);
    }

    #[test]
    fn load_config_rejects_legacy_individual_accounts() {
        with_test_config_dir("legacy_individual_account", || {
            let path = config_path().unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                serde_json::json!({
                    "near_limit_ratio": 0.85,
                    "inactive_threshold_hours": 24,
                    "quiet_hours": {
                        "start_hour": 0,
                        "end_hour": 8
                    },
                    "api": {},
                    "provider_accounts": [
                        {
                            "id": "acct_openai_personal",
                            "provider": "openai",
                            "label": "Personal",
                            "access_mode": "individual"
                        }
                    ]
                })
                .to_string(),
            )
            .unwrap();

            let error = load_config().unwrap_err();
            assert!(error
                .to_string()
                .contains("unsupported individual API account"));
        });
    }

    #[test]
    fn parse_anthropic_oauth_usage_response_full_data() {
        clear_in_memory_anthropic_session();
        update_in_memory_anthropic_session(
            None,
            None,
            None,
            Some("pro".into()),
            Some("premium".into()),
        );

        let value: Value = serde_json::json!({
            "five_hour": { "utilization": 62.5, "resets_at": "2026-03-09T12:00:00Z" },
            "seven_day": { "utilization": 37.0, "resets_at": "2026-03-12T00:00:00Z" }
        });

        let snap = parse_anthropic_oauth_usage_response(&value).unwrap();
        assert_eq!(snap.provider, "anthropic");
        assert_eq!(snap.source, "oauth");
        assert!(snap.account_label.starts_with("Claude"));
        assert_eq!(snap.tokens_in, 63);
        assert!((snap.spent_usd - 37.0).abs() < f64::EPSILON);
        assert!((snap.limit_usd - 100.0).abs() < f64::EPSILON);
        assert_eq!(snap.primary_reset_at.as_deref(), Some("2026-03-09T12:00:00Z"));
        assert_eq!(snap.secondary_reset_at.as_deref(), Some("2026-03-12T00:00:00Z"));
    }

    #[test]
    fn parse_anthropic_oauth_usage_response_falls_back_when_window_missing() {
        clear_in_memory_anthropic_session();
        let value: Value = serde_json::json!({
            "seven_day": { "utilization": 24.0 }
        });

        let snap = parse_anthropic_oauth_usage_response(&value).unwrap();
        assert_eq!(snap.tokens_in, 24);
        assert!((snap.spent_usd - 24.0).abs() < f64::EPSILON);
        assert!(snap.primary_reset_at.is_none());
        assert!(snap.secondary_reset_at.is_none());
    }

    #[test]
    fn parse_openai_oauth_usage_data_extracts_reset_times() {
        let value: Value = serde_json::json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 48.0,
                    "resets_at": "2026-03-10T11:00:00Z"
                },
                "secondary_window": {
                    "used_percent": 21.5,
                    "reset_at": "2026-03-12T00:00:00Z"
                }
            }
        });

        let usage = parse_openai_oauth_usage_data(&value).unwrap();
        assert!((usage.primary_percent - 48.0).abs() < f64::EPSILON);
        assert!((usage.secondary_percent - 21.5).abs() < f64::EPSILON);
        assert_eq!(usage.primary_reset_at.as_deref(), Some("2026-03-10T11:00:00Z"));
        assert_eq!(usage.secondary_reset_at.as_deref(), Some("2026-03-12T00:00:00Z"));
    }

    #[test]
    fn parse_openai_oauth_usage_data_requires_supported_windows() {
        let value: Value = serde_json::json!({
            "plan_type": "plus"
        });

        let error = parse_openai_oauth_usage_data(&value).unwrap_err();
        assert!(error.to_string().contains("quota window"));
    }

    #[test]
    fn parse_anthropic_oauth_usage_data_requires_supported_buckets() {
        let value: Value = serde_json::json!({
            "subscriptionType": "pro"
        });

        let error = parse_anthropic_oauth_usage_data(&value).unwrap_err();
        assert!(error.to_string().contains("utilization buckets"));
    }

    #[test]
    fn failed_oauth_verification_parse_does_not_persist_secrets() {
        with_test_config_dir("oauth_parse_failure", || {
            let value: Value = serde_json::json!({});

            let _ = parse_openai_oauth_usage_data(&value).unwrap_err();

            assert_eq!(SecretStore::load_or_default(), SecretPayload::default());
            assert!(!has_openai_oauth_session());
        });
    }

    #[test]
    fn anthropic_plan_type_ignores_generic_rate_limit_tiers() {
        assert_eq!(
            anthropic_plan_type_from_fields("", "default_claude_ai"),
            None
        );
    }

    #[test]
    fn anthropic_plan_type_falls_back_to_local_claude_credentials() {
        with_claude_credentials_override(
            "local_profile",
            r#"{
                "claudeAiOauth": {
                    "subscriptionType": "pro",
                    "rateLimitTier": "default_claude_ai"
                }
            }"#,
            || {
                let plan_type = get_anthropic_oauth_plan_type();
                let session = anthropic_session().lock().unwrap().clone();

                assert_eq!(plan_type.as_deref(), Some("Pro"));
                assert_eq!(session.subscription_type, "pro");
                assert_eq!(session.rate_limit_tier, "default_claude_ai");
            },
        );
    }

    #[test]
    fn clearing_openai_oauth_preserves_anthropic_secret() {
        let mut payload = SecretPayload::default();
        payload.openai_oauth.refresh_token = "openai-refresh".into();
        payload.anthropic_oauth.refresh_token = "claude-refresh".into();
        payload.anthropic_oauth.subscription_type = "max".into();

        let updated = payload_after_clearing_openai_secret(payload).unwrap();
        assert_eq!(updated.openai_oauth, OpenAiOAuthSecret::default());
        assert_eq!(updated.anthropic_oauth.refresh_token, "claude-refresh");
        assert_eq!(updated.anthropic_oauth.subscription_type, "max");
    }

    #[test]
    fn clearing_anthropic_oauth_preserves_openai_secret() {
        let mut payload = SecretPayload::default();
        payload.openai_oauth.refresh_token = "openai-refresh".into();
        payload.openai_oauth.plan_type = "plus".into();
        payload.anthropic_oauth.refresh_token = "claude-refresh".into();

        let updated = payload_after_clearing_anthropic_secret(payload).unwrap();
        assert_eq!(updated.anthropic_oauth, AnthropicOAuthSecret::default());
        assert_eq!(updated.openai_oauth.refresh_token, "openai-refresh");
        assert_eq!(updated.openai_oauth.plan_type, "plus");
    }
}
