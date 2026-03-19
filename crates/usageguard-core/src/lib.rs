mod secret_store;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Local, Timelike, Utc};
pub use secret_store::SecureStorageStatus;
use secret_store::{app_config_dir, SecretPayload, SecretStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

const KEYRING_SERVICE: &str = "usage-guard";
const CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV: &str = "USAGEGUARD_CLAUDE_CREDENTIALS_PATH_OVERRIDE";
#[cfg(test)]
const OPENAI_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV: &str =
    "USAGEGUARD_OPENAI_LOCAL_USAGE_RESPONSE_OVERRIDE";
#[cfg(test)]
const CLAUDE_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV: &str =
    "USAGEGUARD_CLAUDE_LOCAL_USAGE_RESPONSE_OVERRIDE";
const CODEX_AUTH_PATH_OVERRIDE_ENV: &str = "USAGEGUARD_CODEX_AUTH_PATH_OVERRIDE";
const CODEX_SESSIONS_DIR_OVERRIDE_ENV: &str = "USAGEGUARD_CODEX_SESSIONS_DIR_OVERRIDE";
const CONSUMER_LOCAL_SOURCE: &str = "consumer_local";
const CONSUMER_LOCAL_STATUS_SOURCE: &str = "consumer_local_status";
const CLAUDE_CODE_USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const ANTHROPIC_LOCAL_USAGE_BETA_HEADER: &str = "oauth-2025-04-20";
const CLAUDE_LOCAL_USAGE_CACHE_TTL_SECS: i64 = 300;
const CODEX_WHAM_CACHE_TTL_SECS: i64 = 60;
const CODEX_SESSION_SCAN_FILE_LIMIT: usize = 32;
const CODEX_SESSION_SCAN_LINE_LIMIT: usize = 256;
const CODEX_RATE_LIMIT_STALENESS_THRESHOLD_SECS: u64 = 15 * 60;
pub const DEFAULT_REFRESH_INTERVAL_SECS: u32 = 15;
pub const MIN_REFRESH_INTERVAL_SECS: u32 = 15;
pub const MAX_REFRESH_INTERVAL_SECS: u32 = 900;
const CONSUMER_FIVE_HOUR_NEAR_LIMIT_PERCENT: f64 = 90.0;
const CONSUMER_WEEKLY_NEAR_LIMIT_PERCENT: f64 = 80.0;
const CONSUMER_FIVE_HOUR_UNUSED_PERCENT_MAX: f64 = 20.0;
const CONSUMER_WEEKLY_UNUSED_PERCENT_MAX: f64 = 40.0;
const CONSUMER_FIVE_HOUR_RESET_REMINDER_WINDOW_MINUTES: i64 = 45;
const CONSUMER_WEEKLY_RESET_REMINDER_WINDOW_HOURS: i64 = 24;

fn default_refresh_interval_secs() -> u32 {
    DEFAULT_REFRESH_INTERVAL_SECS
}

fn default_consumer_alerts_enabled() -> bool {
    true
}

pub fn clamp_refresh_interval_secs(value: u32) -> u32 {
    value.clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS)
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
    #[serde(default, rename = "accessToken")]
    access_token: Option<String>,
    #[serde(default, rename = "expiresAt")]
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: Value,
}

#[derive(Debug, Clone, Default)]
struct ClaudeInsightsCacheState {
    fetched_at: Option<DateTime<Utc>>,
    primary_window: Option<ConsumerQuotaWindow>,
    secondary_window: Option<ConsumerQuotaWindow>,
}

#[derive(Debug, Clone, Default)]
struct CodexWhamCacheState {
    fetched_at: Option<DateTime<Utc>>,
    snapshot: Option<UsageSnapshot>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsumerQuotaWindow {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ConsumerQuotaCard {
    #[serde(default)]
    pub primary: Option<ConsumerQuotaWindow>,
    #[serde(default)]
    pub secondary: Option<ConsumerQuotaWindow>,
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
    pub consumer_quota: Option<ConsumerQuotaCard>,
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
    /// Last known widget right-bottom corner in logical pixels [right, bottom].
    /// Saved on quit and restored on next launch.
    #[serde(default)]
    pub widget_position: Option<[f64; 2]>,
    /// User-defined display name for the local Codex consumer connection.
    #[serde(default, alias = "openai_oauth_label")]
    pub openai_consumer_label: Option<String>,
    /// User-defined display name for the local Claude Code consumer connection.
    #[serde(default, alias = "anthropic_oauth_label")]
    pub anthropic_consumer_label: Option<String>,
    /// Whether the UI should display in light mode instead of the default dark mode.
    #[serde(default)]
    pub light_mode: bool,
    /// Whether Codex 5h consumer alerts are enabled.
    #[serde(default = "default_consumer_alerts_enabled", alias = "openai_oauth_5h_alerts_enabled")]
    pub openai_consumer_5h_alerts_enabled: bool,
    /// Whether Codex weekly consumer alerts are enabled.
    #[serde(default = "default_consumer_alerts_enabled", alias = "openai_oauth_week_alerts_enabled")]
    pub openai_consumer_week_alerts_enabled: bool,
    /// Whether Claude Code 5h consumer alerts are enabled.
    #[serde(default = "default_consumer_alerts_enabled", alias = "anthropic_oauth_5h_alerts_enabled")]
    pub anthropic_consumer_5h_alerts_enabled: bool,
    /// Whether Claude Code weekly consumer alerts are enabled.
    #[serde(default = "default_consumer_alerts_enabled", alias = "anthropic_oauth_week_alerts_enabled")]
    pub anthropic_consumer_week_alerts_enabled: bool,
    /// Last release tag that already triggered an update notification.
    #[serde(default)]
    pub last_update_notified_version: Option<String>,
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
            openai_consumer_label: None,
            anthropic_consumer_label: None,
            light_mode: false,
            openai_consumer_5h_alerts_enabled: true,
            openai_consumer_week_alerts_enabled: true,
            anthropic_consumer_5h_alerts_enabled: true,
            anthropic_consumer_week_alerts_enabled: true,
            last_update_notified_version: None,
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

pub fn secure_storage_status() -> SecureStorageStatus {
    SecretStore::status()
}

fn claude_insights_cache() -> &'static Mutex<ClaudeInsightsCacheState> {
    static CACHE: OnceLock<Mutex<ClaudeInsightsCacheState>> = OnceLock::new();

    CACHE.get_or_init(|| Mutex::new(ClaudeInsightsCacheState::default()))
}

fn codex_wham_cache() -> &'static Mutex<CodexWhamCacheState> {
    static CACHE: OnceLock<Mutex<CodexWhamCacheState>> = OnceLock::new();

    CACHE.get_or_init(|| Mutex::new(CodexWhamCacheState::default()))
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

fn codex_auth_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(CODEX_AUTH_PATH_OVERRIDE_ENV) {
        let trimmed = path.trim();

        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    Some(dirs::home_dir()?.join(".codex").join("auth.json"))
}

fn codex_sessions_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(CODEX_SESSIONS_DIR_OVERRIDE_ENV) {
        let trimmed = path.trim();

        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    Some(dirs::home_dir()?.join(".codex").join("sessions"))
}

fn has_local_codex_auth() -> bool {
    let Some(path) = codex_auth_path() else {
        return false;
    };

    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };

    let Ok(auth) = serde_json::from_str::<CodexAuthFile>(&raw) else {
        return false;
    };

    !auth.tokens.is_null()
}

fn collect_jsonl_files(root: &Path, items: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        if metadata.is_dir() {
            collect_jsonl_files(&path, items);

            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        items.push((metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), path));
    }
}

fn value_reset_at(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }

    if let Some(timestamp) = value.as_i64() {
        return DateTime::from_timestamp(timestamp, 0).map(|dt| dt.to_rfc3339());
    }

    None
}

fn consumer_quota_window(used_percent: f64, reset_at: Option<String>) -> ConsumerQuotaWindow {
    ConsumerQuotaWindow {
        available: true,
        used_percent: Some(used_percent.clamp(0.0, 100.0)),
        reset_at,
    }
}

pub fn invalidate_claude_local_insights_cache() {
    *claude_insights_cache().lock().unwrap() = ClaudeInsightsCacheState::default();
}

#[cfg(test)]
fn openai_local_usage_response_override() -> Option<String> {
    std::env::var(OPENAI_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
fn claude_local_usage_response_override() -> Option<String> {
    std::env::var(CLAUDE_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub fn invalidate_codex_wham_cache() {
    *codex_wham_cache().lock().unwrap() = CodexWhamCacheState::default();
}

fn get_valid_claude_code_access_token() -> Option<String> {
    let path = claude_credentials_path()?;

    let raw = fs::read_to_string(path).ok()?;

    let credentials = serde_json::from_str::<ClaudeDesktopCredentials>(&raw).ok()?;

    let oauth = credentials.claude_ai_oauth;

    let access_token = oauth.access_token.filter(|token| !token.trim().is_empty())?;

    if let Some(expires_at_ms) = oauth.expires_at_ms {
        let now_ms = Utc::now().timestamp_millis();

        if expires_at_ms < now_ms + 60_000 {
            return None;
        }
    }

    Some(access_token)
}

fn fetch_claude_code_usage_from_api(
    access_token: &str,
) -> Option<(ConsumerQuotaWindow, Option<ConsumerQuotaWindow>)> {
    let value = match fetch_claude_local_usage_value(access_token) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[usageguard] claude local usage fetch failed: {error:?}");

            return None;
        }
    };

    match parse_claude_local_usage_response(&value) {
        Ok(windows) => Some(windows),
        Err(error) => {
            eprintln!("[usageguard] claude local usage parse failed: {error}");

            None
        }
    }
}

fn fetch_claude_local_quota_windows() -> Option<(ConsumerQuotaWindow, Option<ConsumerQuotaWindow>)>
{
    let now = Utc::now();

    {
        let cache = claude_insights_cache().lock().unwrap();

        if let Some(fetched_at) = cache.fetched_at {
            if now.signed_duration_since(fetched_at)
                < Duration::seconds(CLAUDE_LOCAL_USAGE_CACHE_TTL_SECS)
            {
                return cache
                    .primary_window
                    .clone()
                    .map(|p| (p, cache.secondary_window.clone()));
            }
        }
    }

    let (primary_window, secondary_window) = match get_valid_claude_code_access_token()
        .as_deref()
        .and_then(fetch_claude_code_usage_from_api)
    {
        Some((p, s)) => (Some(p), s),
        None => (None, None),
    };

    *claude_insights_cache().lock().unwrap() = ClaudeInsightsCacheState {
        fetched_at: Some(now),
        primary_window: primary_window.clone(),
        secondary_window: secondary_window.clone(),
    };

    primary_window.map(|p| (p, secondary_window))
}

fn openai_consumer_plan_label(value: &str) -> String {
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

fn codex_plan_type_from_payload(payload: &Value) -> Option<String> {
    pick_str(payload, &["plan_type", "planType"])
        .filter(|value| is_non_empty(value) && !value.eq_ignore_ascii_case("unknown"))
        .map(openai_consumer_plan_label)
}

fn codex_account_label(plan_type: Option<&str>) -> String {
    match plan_type.filter(|value| is_non_empty(value)) {
        Some(plan_type) => format!("Codex {}", openai_consumer_plan_label(plan_type)),
        None => "Codex".to_string(),
    }
}

fn codex_plan_type_from_snapshot(snapshot: &UsageSnapshot) -> Option<String> {
    snapshot
        .account_label
        .strip_prefix("Codex ")
        .map(str::trim)
        .filter(|value| is_non_empty(value))
        .map(str::to_string)
}

fn codex_rate_limits(payload: &Value) -> Option<&Value> {
    let rate_limits = payload.get("rate_limits")?;

    (rate_limits.is_object()
        && rate_limits.get("primary").is_some_and(Value::is_object)
        && rate_limits.get("secondary").is_some_and(Value::is_object))
    .then_some(rate_limits)
}

fn latest_codex_rate_limit_payload() -> Option<(Value, SystemTime)> {
    let root = codex_sessions_dir()?;

    if !root.exists() {
        return None;
    }

    let mut files = vec![];

    collect_jsonl_files(&root, &mut files);

    files.sort_by(|left, right| right.0.cmp(&left.0));

    let mut fallback_entry: Option<(Value, SystemTime)> = None;

    for (modified, path) in files.into_iter().take(CODEX_SESSION_SCAN_FILE_LIMIT) {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };

        for line in raw.lines().rev().take(CODEX_SESSION_SCAN_LINE_LIMIT) {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };

            let Some(payload) = value.get("payload") else {
                continue;
            };

            if pick_str(payload, &["type"]) != Some("token_count") {
                continue;
            }

            let Some(rate_limits) = codex_rate_limits(payload) else {
                continue;
            };

            if pick_str(rate_limits, &["limit_id"]) == Some("codex") {
                return Some((payload.clone(), modified));
            }

            if fallback_entry.is_none() {
                fallback_entry = Some((payload.clone(), modified));
            }
        }
    }

    fallback_entry
}

fn codex_rate_limit_is_fresh(modified: SystemTime) -> bool {
    modified
        .elapsed()
        .unwrap_or(std::time::Duration::MAX)
        .as_secs()
        <= CODEX_RATE_LIMIT_STALENESS_THRESHOLD_SECS
}

fn build_claude_local_consumer_snapshot(
    primary_window: ConsumerQuotaWindow,
    secondary_window: Option<ConsumerQuotaWindow>,
) -> UsageSnapshot {
    let plan_type = get_anthropic_consumer_plan_type();
    let primary_used = primary_window.used_percent.unwrap_or(0.0);
    let primary_reset_at = primary_window.reset_at.clone();
    let secondary_reset_at = secondary_window.as_ref().and_then(|w| w.reset_at.clone());

    UsageSnapshot {
        provider: "anthropic".into(),
        account_label: match plan_type {
            Some(plan_type) if is_non_empty(&plan_type) => format!("Claude Code {plan_type}"),
            _ => "Claude Code".to_string(),
        },
        spent_usd: 0.0,
        limit_usd: 0.0,
        tokens_in: primary_used.round() as u64,
        tokens_out: 0,
        inactive_hours: 0,
        source: CONSUMER_LOCAL_SOURCE.to_string(),
        status_code: Some("consumer_local_quota".to_string()),
        status_message: None,
        api_metrics: None,
        consumer_quota: Some(ConsumerQuotaCard {
            primary: Some(primary_window),
            secondary: secondary_window,
        }),
        primary_reset_at,
        secondary_reset_at,
    }
}

fn load_local_claude_consumer_metadata() -> Option<(String, String)> {
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

fn anthropic_plan_type_from_fields(
    subscription_type: &str,
    rate_limit_tier: &str,
) -> Option<String> {
    anthropic_plan_label_from_subscription_type(subscription_type)
        .or_else(|| anthropic_plan_label_from_rate_limit_tier(rate_limit_tier))
}

pub fn has_openai_consumer_source() -> bool {
    has_local_codex_auth() || latest_codex_rate_limit_payload().is_some()
}

pub fn has_openai_consumer_usage() -> bool {
    latest_codex_rate_limit_payload()
        .is_some_and(|(_, modified)| codex_rate_limit_is_fresh(modified))
        || codex_wham_cache().lock().unwrap().snapshot.is_some()
}

pub fn get_openai_consumer_plan_type() -> Option<String> {
    latest_codex_rate_limit_payload()
        .as_ref()
        .and_then(|(payload, _)| codex_rate_limits(payload))
        .and_then(codex_plan_type_from_payload)
        .or_else(|| {
            codex_wham_cache()
                .lock()
                .unwrap()
                .snapshot
                .as_ref()
                .and_then(codex_plan_type_from_snapshot)
        })
}

pub fn has_anthropic_consumer_source() -> bool {
    claude_credentials_path().is_some_and(|path| path.exists())
}

pub fn has_anthropic_consumer_5h_usage() -> bool {
    if !has_anthropic_consumer_source() {
        return false;
    }

    claude_insights_cache()
        .lock()
        .unwrap()
        .primary_window
        .is_some()
}

pub fn has_anthropic_consumer_week_usage() -> bool {
    false
}

pub fn has_anthropic_consumer_usage() -> bool {
    has_anthropic_consumer_5h_usage()
}

pub fn get_anthropic_consumer_plan_type() -> Option<String> {
    load_local_claude_consumer_metadata().and_then(|(subscription_type, rate_limit_tier)| {
        anthropic_plan_type_from_fields(&subscription_type, &rate_limit_tier)
    })
}

fn parse_codex_local_usage_payload(payload: &Value) -> Result<UsageSnapshot> {
    let rate_limits = payload
        .get("rate_limits")
        .context("Codex session entry missing rate_limits")?;

    let primary = rate_limits
        .get("primary")
        .context("Codex session entry missing primary rate limit")?;

    let secondary = rate_limits
        .get("secondary")
        .context("Codex session entry missing secondary rate limit")?;

    let primary_percent = pick_f64(primary, &["used_percent"])
        .context("Codex session entry missing primary used_percent")?
        .clamp(0.0, 100.0);

    let secondary_percent = pick_f64(secondary, &["used_percent"])
        .context("Codex session entry missing secondary used_percent")?
        .clamp(0.0, 100.0);

    let plan_type = codex_plan_type_from_payload(rate_limits);

    Ok(UsageSnapshot {
        provider: "openai".into(),
        account_label: codex_account_label(plan_type.as_deref()),
        spent_usd: secondary_percent,
        limit_usd: 100.0,
        tokens_in: primary_percent.round() as u64,
        tokens_out: 0,
        inactive_hours: 0,
        source: CONSUMER_LOCAL_SOURCE.to_string(),
        status_code: None,
        status_message: None,
        api_metrics: None,
        consumer_quota: Some(ConsumerQuotaCard {
            primary: Some(consumer_quota_window(
                primary_percent,
                primary
                    .get("resets_at")
                    .and_then(value_reset_at)
                    .or_else(|| primary.get("reset_at").and_then(value_reset_at)),
            )),
            secondary: Some(consumer_quota_window(
                secondary_percent,
                secondary
                    .get("resets_at")
                    .and_then(value_reset_at)
                    .or_else(|| secondary.get("reset_at").and_then(value_reset_at)),
            )),
        }),
        primary_reset_at: primary
            .get("resets_at")
            .and_then(value_reset_at)
            .or_else(|| primary.get("reset_at").and_then(value_reset_at)),
        secondary_reset_at: secondary
            .get("resets_at")
            .and_then(value_reset_at)
            .or_else(|| secondary.get("reset_at").and_then(value_reset_at)),
    })
}

#[derive(Debug, Clone)]
struct CodexWhamUsageData {
    plan_type: String,
    primary_percent: f64,
    secondary_percent: f64,
    primary_reset_at: Option<String>,
    secondary_reset_at: Option<String>,
}

fn get_codex_auth_tokens() -> Option<(String, String)> {
    let path = codex_auth_path()?;

    let raw = fs::read_to_string(path).ok()?;

    let value = serde_json::from_str::<Value>(&raw).ok()?;

    let access_token = value
        .pointer("/tokens/access_token")
        .and_then(|entry| entry.as_str())
        .filter(|value| !value.trim().is_empty())?;

    let account_id = value
        .pointer("/tokens/account_id")
        .and_then(|entry| entry.as_str())
        .unwrap_or_default();

    Some((access_token.to_string(), account_id.to_string()))
}

fn openai_local_usage_window_reset_at(
    value: &Value,
    primary_key: &str,
    camel_key: &str,
    fallback_key: &str,
) -> Option<String> {
    let paths = [
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
    ];

    paths.iter().find_map(|pointer| {
        let entry = value.pointer(pointer)?;

        value_reset_at(entry)
    })
}

fn parse_openai_local_usage_data(value: &Value) -> Result<CodexWhamUsageData> {
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
            "OpenAI local usage response missing supported quota window data"
        ));
    }

    let primary_reset_at = openai_local_usage_window_reset_at(
        value,
        "primary_window",
        "primaryWindow",
        "short_window",
    );

    let secondary_reset_at = openai_local_usage_window_reset_at(
        value,
        "secondary_window",
        "secondaryWindow",
        "long_window",
    );

    Ok(CodexWhamUsageData {
        plan_type: openai_consumer_plan_label(
            value
                .get("plan_type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
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

fn fetch_codex_wham_usage_live() -> Option<UsageSnapshot> {
    let (access_token, account_id) = get_codex_auth_tokens()?;

    let value = match fetch_openai_local_usage_value(&access_token, &account_id) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[usageguard] codex local usage fetch failed: {error:?}");

            return None;
        }
    };

    let usage = match parse_openai_local_usage_data(&value) {
        Ok(usage) => usage,
        Err(error) => {
            eprintln!("[usageguard] codex local usage parse failed: {error}");

            return None;
        }
    };

    let plan_type = is_non_empty(&usage.plan_type).then_some(usage.plan_type.as_str());

    Some(UsageSnapshot {
        provider: "openai".into(),
        account_label: codex_account_label(plan_type),
        spent_usd: usage.secondary_percent,
        limit_usd: 100.0,
        tokens_in: usage.primary_percent.round() as u64,
        tokens_out: 0,
        inactive_hours: 0,
        source: CONSUMER_LOCAL_SOURCE.to_string(),
        status_code: None,
        status_message: None,
        api_metrics: None,
        consumer_quota: Some(ConsumerQuotaCard {
            primary: Some(ConsumerQuotaWindow {
                available: true,
                used_percent: Some(usage.primary_percent),
                reset_at: usage.primary_reset_at.clone(),
            }),
            secondary: Some(ConsumerQuotaWindow {
                available: true,
                used_percent: Some(usage.secondary_percent),
                reset_at: usage.secondary_reset_at.clone(),
            }),
        }),
        primary_reset_at: usage.primary_reset_at,
        secondary_reset_at: usage.secondary_reset_at,
    })
}

fn fetch_codex_wham_usage() -> Option<UsageSnapshot> {
    let now = Utc::now();

    {
        let cache = codex_wham_cache().lock().unwrap();

        if let Some(fetched_at) = cache.fetched_at {
            if now.signed_duration_since(fetched_at) < Duration::seconds(CODEX_WHAM_CACHE_TTL_SECS)
            {
                return cache.snapshot.clone();
            }
        }
    }

    let snapshot = fetch_codex_wham_usage_live();

    *codex_wham_cache().lock().unwrap() = CodexWhamCacheState {
        fetched_at: Some(now),
        snapshot: snapshot.clone(),
    };

    snapshot
}

pub fn fetch_openai_consumer_usage() -> Option<UsageSnapshot> {
    // Prefer fresh local JSONL data written by Codex within the last 15 minutes.
    if let Some((payload, modified)) = latest_codex_rate_limit_payload() {
        if codex_rate_limit_is_fresh(modified) {
            match parse_codex_local_usage_payload(&payload) {
                Ok(snapshot) => return Some(snapshot),
                Err(error) => {
                    eprintln!("[usageguard] codex local usage parse failed: {error}");
                }
            }
        }
    }

    if let Some(snapshot) = fetch_codex_wham_usage() {
        return Some(snapshot);
    }

    if !has_openai_consumer_source() {
        return None;
    }

    Some(error_snapshot(
        "openai",
        &codex_account_label(get_openai_consumer_plan_type().as_deref()),
        CONSUMER_LOCAL_STATUS_SOURCE,
        Some("consumer_local_waiting_for_usage"),
        Some("Codex is signed in locally. Usage appears after your next Codex request."),
    ))
}

fn claude_local_usage_bucket<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn claude_local_usage_bucket_value(bucket: &Value) -> Option<f64> {
    let value = match bucket {
        Value::Number(number) => number.as_f64(),
        Value::Object(_) => pick_f64(bucket, &["utilization", "usage", "percent", "value"]),
        _ => None,
    }?;

    if !value.is_finite() || value < 0.0 {
        return None;
    }

    let percent = if value <= 1.0 { value * 100.0 } else { value };

    Some(percent.clamp(0.0, 100.0))
}

fn claude_local_usage_bucket_percent(value: &Value, keys: &[&str]) -> Option<f64> {
    claude_local_usage_bucket(value, keys)
        .and_then(claude_local_usage_bucket_value)
        .or_else(|| {
            keys.iter().find_map(|key| {
                let utilization_key = format!("{key}_utilization");

                value
                    .get(utilization_key.as_str())
                    .and_then(claude_local_usage_bucket_value)
            })
        })
}

fn claude_local_usage_bucket_reset_at(value: &Value, keys: &[&str]) -> Option<String> {
    claude_local_usage_bucket(value, keys).and_then(|bucket| {
        pick_str(bucket, &["resets_at", "reset_at", "resetsAt", "resetAt"])
            .map(str::to_string)
            .or_else(|| {
                ["resets_at", "reset_at", "resetsAt", "resetAt"]
                    .iter()
                    .find_map(|key| bucket.get(*key).and_then(value_reset_at))
            })
    })
}

fn parse_claude_local_usage_response(
    value: &Value,
) -> Result<(ConsumerQuotaWindow, Option<ConsumerQuotaWindow>)> {
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

    let five_hour_percent = claude_local_usage_bucket_percent(value, five_hour_keys);
    let seven_day_percent = claude_local_usage_bucket_percent(value, seven_day_keys);

    if five_hour_percent.is_none() && seven_day_percent.is_none() {
        return Err(anyhow!(
            "Claude local usage response missing supported utilization buckets"
        ));
    }

    let primary_used = five_hour_percent.or(seven_day_percent).unwrap_or(0.0);
    let primary_reset_at = claude_local_usage_bucket_reset_at(value, five_hour_keys)
        .or_else(|| claude_local_usage_bucket_reset_at(value, seven_day_keys));

    let primary_window = consumer_quota_window(primary_used, primary_reset_at);

    let secondary_window = seven_day_percent.map(|used_percent| {
        consumer_quota_window(
            used_percent,
            claude_local_usage_bucket_reset_at(value, seven_day_keys),
        )
    });

    Ok((primary_window, secondary_window))
}

pub fn fetch_anthropic_consumer_usage() -> Option<UsageSnapshot> {
    if !has_anthropic_consumer_source() {
        return None;
    }

    if let Some((primary_window, secondary_window)) = fetch_claude_local_quota_windows() {
        return Some(build_claude_local_consumer_snapshot(
            primary_window,
            secondary_window,
        ));
    }

    Some(error_snapshot(
        "anthropic",
        &match get_anthropic_consumer_plan_type() {
            Some(plan_type) if is_non_empty(&plan_type) => format!("Claude Code {plan_type}"),
            _ => "Claude Code".to_string(),
        },
        CONSUMER_LOCAL_STATUS_SOURCE,
        Some("consumer_local_usage_pending"),
        Some("Fetching Claude Code 5h quota… (appears ~10 s after app launch)"),
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

    if !changed && !cleanup_needed {
        return Ok(false);
    }

    if changed {
        save_secret_payload(&payload)?;
    }

    for key_id in migrated_keyring_ids {
        delete_keyring_password(&key_id);
    }

    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, "openai.oauth.tokens") {
        let _ = entry.delete_credential();
    }

    if let Ok(path) = app_config_dir().map(|dir| dir.join("oauth_tokens.json")) {
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

fn migrate_legacy_consumer_alert_preferences(raw: &Value, cfg: &mut AppConfig) -> bool {
    let mut migrated = false;

    for (legacy_key, short_key, week_key, short_value, week_value) in [
        (
            "openai_oauth_alerts_enabled",
            "openai_consumer_5h_alerts_enabled",
            "openai_consumer_week_alerts_enabled",
            &mut cfg.openai_consumer_5h_alerts_enabled,
            &mut cfg.openai_consumer_week_alerts_enabled,
        ),
        (
            "anthropic_oauth_alerts_enabled",
            "anthropic_consumer_5h_alerts_enabled",
            "anthropic_consumer_week_alerts_enabled",
            &mut cfg.anthropic_consumer_5h_alerts_enabled,
            &mut cfg.anthropic_consumer_week_alerts_enabled,
        ),
    ] {
        let legacy = raw.get(legacy_key).and_then(|value| value.as_bool());

        let has_short = raw.get(short_key).is_some();

        let has_week = raw.get(week_key).is_some();

        if legacy.is_some() && (!has_short || !has_week) {
            migrated = true;
        }

        let Some(enabled) = legacy else {
            continue;
        };

        if !has_short {
            *short_value = enabled;
        }

        if !has_week {
            *week_value = enabled;
        }
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

    let mut cfg = serde_json::from_value::<AppConfig>(raw_value.clone())
        .with_context(|| format!("Invalid config JSON: {}", path.display()))?;

    let mut migrated = false;

    migrated |= migrate_secret_payload(&mut cfg)?;

    migrated |= migrate_legacy_provider_accounts(&mut cfg);

    migrated |= migrate_legacy_consumer_alert_preferences(&raw_value, &mut cfg);

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

fn snapshot_primary_quota(snapshot: &UsageSnapshot) -> Option<(f64, Option<&str>)> {
    if let Some(window) = snapshot
        .consumer_quota
        .as_ref()
        .and_then(|quota| quota.primary.as_ref())
    {
        if window.available {
            return window
                .used_percent
                .map(|used_percent| (used_percent, window.reset_at.as_deref()));
        }

        return None;
    }

    Some((
        snapshot.tokens_in as f64,
        snapshot.primary_reset_at.as_deref(),
    ))
}

fn snapshot_secondary_quota(snapshot: &UsageSnapshot) -> Option<(f64, Option<&str>)> {
    if let Some(window) = snapshot
        .consumer_quota
        .as_ref()
        .and_then(|quota| quota.secondary.as_ref())
    {
        if window.available {
            return window
                .used_percent
                .map(|used_percent| (used_percent, window.reset_at.as_deref()));
        }

        return None;
    }

    let has_legacy_week_window = snapshot.limit_usd > 0.0
        || snapshot.spent_usd > 0.0
        || snapshot.secondary_reset_at.is_some();

    has_legacy_week_window.then_some((snapshot.spent_usd, snapshot.secondary_reset_at.as_deref()))
}

fn quota_percent_left(used_percent: f64) -> u32 {
    (100.0 - used_percent.clamp(0.0, 100.0)).round() as u32
}

fn parse_reset_at(value: Option<&str>) -> Option<DateTime<Utc>> {
    let value = value?.trim();

    if value.is_empty() {
        return None;
    }

    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn time_until_reset(now: DateTime<Utc>, reset_at: Option<&str>) -> Option<Duration> {
    let reset_at = parse_reset_at(reset_at)?;

    let remaining = reset_at.signed_duration_since(now);

    (remaining > Duration::zero()).then_some(remaining)
}

fn format_time_until_reset(duration: Duration) -> String {
    let total_minutes = duration.num_minutes().max(1);

    let hours = total_minutes / 60;

    let minutes = total_minutes % 60;

    match (hours, minutes) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

fn push_consumer_window_alerts(
    alerts: &mut Vec<Alert>,
    now: DateTime<Utc>,
    window_label: &str,
    used_percent: f64,
    reset_at: Option<&str>,
    near_limit_percent: f64,
    unused_percent_max: f64,
    reminder_window: Duration,
    exhausted_code: &str,
    near_limit_code: &str,
    reminder_code: &str,
) {
    let used_percent = used_percent.clamp(0.0, 100.0);

    let used_display = used_percent.round() as u32;

    let left_display = quota_percent_left(used_percent);

    if used_percent >= 100.0 {
        alerts.push(Alert {
            level: "critical".into(),
            code: exhausted_code.into(),
            message: format!(
                "{window_label} quota exhausted: {used_display}% used, {left_display}% left"
            ),
        });
    } else if used_percent >= near_limit_percent {
        alerts.push(Alert {
            level: "warning".into(),
            code: near_limit_code.into(),
            message: format!(
                "{window_label} quota nearly used up: {used_display}% used, {left_display}% left"
            ),
        });
    }

    if used_percent > unused_percent_max {
        return;
    }

    let Some(remaining) = time_until_reset(now, reset_at) else {
        return;
    };

    if remaining > reminder_window {
        return;
    }

    alerts.push(Alert {
        level: "info".into(),
        code: reminder_code.into(),
        message: format!(
            "{window_label} quota resets in {} and only {used_display}% has been used",
            format_time_until_reset(remaining)
        ),
    });
}

fn evaluate_consumer_alerts(
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
    cfg: &AppConfig,
) -> Vec<Alert> {
    let mut alerts = vec![];

    let primary_quota = snapshot_primary_quota(snapshot);

    let secondary_quota = snapshot_secondary_quota(snapshot);

    match snapshot.provider.as_str() {
        "openai" => {
            if cfg.openai_consumer_5h_alerts_enabled {
                if let Some((used_percent, reset_at)) = primary_quota {
                    push_consumer_window_alerts(
                        &mut alerts,
                        now,
                        "5h",
                        used_percent,
                        reset_at,
                        CONSUMER_FIVE_HOUR_NEAR_LIMIT_PERCENT,
                        CONSUMER_FIVE_HOUR_UNUSED_PERCENT_MAX,
                        Duration::minutes(CONSUMER_FIVE_HOUR_RESET_REMINDER_WINDOW_MINUTES),
                        "quota_5h_exhausted",
                        "quota_5h_near_limit",
                        "quota_5h_unused_before_reset",
                    );
                }
            }

            if cfg.openai_consumer_week_alerts_enabled {
                if let Some((used_percent, reset_at)) = secondary_quota {
                    push_consumer_window_alerts(
                        &mut alerts,
                        now,
                        "Week",
                        used_percent,
                        reset_at,
                        CONSUMER_WEEKLY_NEAR_LIMIT_PERCENT,
                        CONSUMER_WEEKLY_UNUSED_PERCENT_MAX,
                        Duration::hours(CONSUMER_WEEKLY_RESET_REMINDER_WINDOW_HOURS),
                        "quota_week_exhausted",
                        "quota_week_near_limit",
                        "quota_week_unused_before_reset",
                    );
                }
            }
        }
        "anthropic" => {
            if cfg.anthropic_consumer_5h_alerts_enabled {
                if let Some((used_percent, reset_at)) = primary_quota {
                    push_consumer_window_alerts(
                        &mut alerts,
                        now,
                        "5h",
                        used_percent,
                        reset_at,
                        CONSUMER_FIVE_HOUR_NEAR_LIMIT_PERCENT,
                        CONSUMER_FIVE_HOUR_UNUSED_PERCENT_MAX,
                        Duration::minutes(CONSUMER_FIVE_HOUR_RESET_REMINDER_WINDOW_MINUTES),
                        "quota_5h_exhausted",
                        "quota_5h_near_limit",
                        "quota_5h_unused_before_reset",
                    );
                }
            }

            if cfg.anthropic_consumer_week_alerts_enabled {
                if let Some((used_percent, reset_at)) = secondary_quota {
                    push_consumer_window_alerts(
                        &mut alerts,
                        now,
                        "Week",
                        used_percent,
                        reset_at,
                        CONSUMER_WEEKLY_NEAR_LIMIT_PERCENT,
                        CONSUMER_WEEKLY_UNUSED_PERCENT_MAX,
                        Duration::hours(CONSUMER_WEEKLY_RESET_REMINDER_WINDOW_HOURS),
                        "quota_week_exhausted",
                        "quota_week_near_limit",
                        "quota_week_unused_before_reset",
                    );
                }
            }
        }
        _ => {
            if let Some((used_percent, reset_at)) = primary_quota {
                push_consumer_window_alerts(
                    &mut alerts,
                    now,
                    "5h",
                    used_percent,
                    reset_at,
                    CONSUMER_FIVE_HOUR_NEAR_LIMIT_PERCENT,
                    CONSUMER_FIVE_HOUR_UNUSED_PERCENT_MAX,
                    Duration::minutes(CONSUMER_FIVE_HOUR_RESET_REMINDER_WINDOW_MINUTES),
                    "quota_5h_exhausted",
                    "quota_5h_near_limit",
                    "quota_5h_unused_before_reset",
                );
            }

            if let Some((used_percent, reset_at)) = secondary_quota {
                push_consumer_window_alerts(
                    &mut alerts,
                    now,
                    "Week",
                    used_percent,
                    reset_at,
                    CONSUMER_WEEKLY_NEAR_LIMIT_PERCENT,
                    CONSUMER_WEEKLY_UNUSED_PERCENT_MAX,
                    Duration::hours(CONSUMER_WEEKLY_RESET_REMINDER_WINDOW_HOURS),
                    "quota_week_exhausted",
                    "quota_week_near_limit",
                    "quota_week_unused_before_reset",
                );
            }
        }
    }

    alerts
}

fn evaluate_standard_alerts(snapshot: &UsageSnapshot, cfg: &AppConfig) -> Vec<Alert> {
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

pub fn evaluate_alerts(
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
    cfg: &AppConfig,
) -> Vec<Alert> {
    if snapshot.source == CONSUMER_LOCAL_SOURCE {
        evaluate_consumer_alerts(snapshot, now, cfg)
    } else {
        evaluate_standard_alerts(snapshot, cfg)
    }
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

pub fn should_notify_alert(alert: &Alert, now: DateTime<Local>, cfg: &AppConfig) -> bool {
    alert.level == "critical" || !is_quiet_hour(now, &cfg.quiet_hours)
}

pub fn should_notify(alerts: &[Alert], now: DateTime<Local>, cfg: &AppConfig) -> bool {
    alerts
        .iter()
        .any(|alert| should_notify_alert(alert, now, cfg))
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

    // Local consumer app sources first.
    if let Some(mut s) = fetch_openai_consumer_usage() {
        if let Some(label) = cfg
            .openai_consumer_label
            .as_deref()
            .filter(|l| !l.trim().is_empty())
        {
            s.account_label = label.to_string();
        }

        items.push(s);
    }

    if let Some(mut s) = fetch_anthropic_consumer_usage() {
        if let Some(label) = cfg
            .anthropic_consumer_label
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
        ApiFetchError::Transport(_) => VerificationError::new(
            VerificationErrorKind::UpstreamUnavailable,
            unavailable_message,
        ),
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

fn fetch_openai_local_usage_value(
    access_token: &str,
    account_id: &str,
) -> std::result::Result<Value, ApiFetchError> {
    #[cfg(test)]
    if let Some(raw) = openai_local_usage_response_override() {
        return serde_json::from_str(&raw).map_err(|error| ApiFetchError::InvalidResponse(error.into()));
    }

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

fn fetch_claude_local_usage_value(
    access_token: &str,
) -> std::result::Result<Value, ApiFetchError> {
    #[cfg(test)]
    if let Some(raw) = claude_local_usage_response_override() {
        return serde_json::from_str(&raw).map_err(|error| ApiFetchError::InvalidResponse(error.into()));
    }

    let client = client_with_timeout().map_err(ApiFetchError::Transport)?;

    let resp = client
        .get(CLAUDE_CODE_USAGE_API_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("anthropic-beta", ANTHROPIC_LOCAL_USAGE_BETA_HEADER)
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
        consumer_quota: None,
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
        consumer_quota: None,
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
        consumer_quota: None,
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
        consumer_quota: None,
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
        consumer_quota: None,
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
        consumer_quota: None,
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
            consumer_quota: None,
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
            consumer_quota: None,
            primary_reset_at: None,
            secondary_reset_at: None,
        },
    ]
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

    fn with_codex_consumer_override(
        name: &str,
        auth_body: &str,
        session_lines: &[&str],
        test: impl FnOnce(),
    ) {
        with_codex_local_usage_override(name, auth_body, session_lines, None, test);
    }

    fn with_codex_local_usage_override(
        name: &str,
        auth_body: &str,
        session_lines: &[&str],
        usage_response: Option<&str>,
        test: impl FnOnce(),
    ) {
        let _guard = crate::secret_store::test_env_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let root = std::env::temp_dir().join(format!(
            "usageguard_core_codex_consumer_{name}_{}",
            std::process::id()
        ));

        let config_root = root.join("config");

        let auth_path = root.join(".codex").join("auth.json");

        let session_path = root.join(".codex").join("sessions").join("test.jsonl");

        let _ = fs::remove_dir_all(&root);

        fs::create_dir_all(&config_root).unwrap();

        fs::create_dir_all(auth_path.parent().unwrap()).unwrap();

        fs::create_dir_all(session_path.parent().unwrap()).unwrap();

        fs::write(&auth_path, auth_body).unwrap();

        fs::write(&session_path, session_lines.join("\n")).unwrap();

        std::env::set_var("USAGEGUARD_CONFIG_DIR_OVERRIDE", &config_root);

        std::env::set_var(CODEX_AUTH_PATH_OVERRIDE_ENV, &auth_path);

        std::env::set_var(
            CODEX_SESSIONS_DIR_OVERRIDE_ENV,
            session_path.parent().unwrap(),
        );

        match usage_response {
            Some(raw) => std::env::set_var(OPENAI_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV, raw),
            None => std::env::remove_var(OPENAI_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV),
        }

        invalidate_claude_local_insights_cache();
        invalidate_codex_wham_cache();

        let _ = SecretStore::clear();

        let result = catch_unwind(AssertUnwindSafe(test));

        let _ = SecretStore::clear();

        invalidate_claude_local_insights_cache();
        invalidate_codex_wham_cache();

        std::env::remove_var("USAGEGUARD_CONFIG_DIR_OVERRIDE");

        std::env::remove_var(CODEX_AUTH_PATH_OVERRIDE_ENV);

        std::env::remove_var(CODEX_SESSIONS_DIR_OVERRIDE_ENV);

        std::env::remove_var(OPENAI_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV);

        let _ = fs::remove_dir_all(&root);

        if let Err(payload) = result {
            resume_unwind(payload);
        }
    }

    fn with_claude_credentials_override(name: &str, body: &str, test: impl FnOnce()) {
        let _guard = crate::secret_store::test_env_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

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

        invalidate_claude_local_insights_cache();

        let _ = SecretStore::clear();

        let result = catch_unwind(AssertUnwindSafe(test));

        let _ = SecretStore::clear();

        invalidate_claude_local_insights_cache();

        std::env::remove_var("USAGEGUARD_CONFIG_DIR_OVERRIDE");

        std::env::remove_var(CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV);

        let _ = fs::remove_dir_all(&root);

        if let Err(payload) = result {
            resume_unwind(payload);
        }
    }

    fn with_claude_local_override(
        name: &str,
        credentials_body: &str,
        usage_response: Option<&str>,
        test: impl FnOnce(),
    ) {
        let _guard = crate::secret_store::test_env_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let root = std::env::temp_dir().join(format!(
            "usageguard_core_claude_local_{name}_{}",
            std::process::id()
        ));

        let config_root = root.join("config");

        let credentials_path = root.join(".claude").join(".credentials.json");

        let _ = fs::remove_dir_all(&root);

        fs::create_dir_all(&config_root).unwrap();

        fs::create_dir_all(credentials_path.parent().unwrap()).unwrap();

        fs::write(&credentials_path, credentials_body).unwrap();

        std::env::set_var("USAGEGUARD_CONFIG_DIR_OVERRIDE", &config_root);

        std::env::set_var(CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV, &credentials_path);

        match usage_response {
            Some(raw) => std::env::set_var(CLAUDE_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV, raw),
            None => std::env::remove_var(CLAUDE_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV),
        }

        invalidate_codex_wham_cache();
        invalidate_claude_local_insights_cache();

        let _ = SecretStore::clear();

        let result = catch_unwind(AssertUnwindSafe(test));

        let _ = SecretStore::clear();

        invalidate_codex_wham_cache();
        invalidate_claude_local_insights_cache();

        std::env::remove_var("USAGEGUARD_CONFIG_DIR_OVERRIDE");

        std::env::remove_var(CLAUDE_CREDENTIALS_PATH_OVERRIDE_ENV);

        std::env::remove_var(CLAUDE_LOCAL_USAGE_RESPONSE_OVERRIDE_ENV);

        let _ = fs::remove_dir_all(&root);

        if let Err(payload) = result {
            resume_unwind(payload);
        }
    }

    fn with_test_config_dir(name: &str, test: impl FnOnce()) {
        let _guard = crate::secret_store::test_env_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let root = std::env::temp_dir().join(format!(
            "usageguard_core_validation_{name}_{}",
            std::process::id()
        ));

        let config_root = root.join("config");

        let _ = fs::remove_dir_all(&root);

        fs::create_dir_all(&config_root).unwrap();

        std::env::set_var("USAGEGUARD_CONFIG_DIR_OVERRIDE", &config_root);

        invalidate_codex_wham_cache();
        invalidate_claude_local_insights_cache();

        let _ = SecretStore::clear();

        let result = catch_unwind(AssertUnwindSafe(test));

        invalidate_codex_wham_cache();
        invalidate_claude_local_insights_cache();

        let _ = SecretStore::clear();

        std::env::remove_var("USAGEGUARD_CONFIG_DIR_OVERRIDE");

        let _ = fs::remove_dir_all(&root);

        if let Err(payload) = result {
            resume_unwind(payload);
        }
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
            consumer_quota: None,
            primary_reset_at: None,
            secondary_reset_at: None,
        };

        let alerts = evaluate_alerts(&s, Utc::now(), &cfg);

        assert!(alerts.iter().any(|a| a.code == "near_limit"));
    }

    fn consumer_snapshot(
        five_hour_used: u64,
        week_used: f64,
        five_hour_reset_at: Option<&str>,
        week_reset_at: Option<&str>,
    ) -> UsageSnapshot {
        UsageSnapshot {
            provider: "openai".into(),
            account_label: "ChatGPT Plus".into(),
            spent_usd: week_used,
            limit_usd: 100.0,
            tokens_in: five_hour_used,
            tokens_out: 0,
            inactive_hours: 0,
            source: CONSUMER_LOCAL_SOURCE.into(),
            status_code: None,
            status_message: None,
            api_metrics: None,
            consumer_quota: None,
            primary_reset_at: five_hour_reset_at.map(str::to_string),
            secondary_reset_at: week_reset_at.map(str::to_string),
        }
    }

    #[test]

    fn consumer_five_hour_near_limit_alert_fires_at_threshold() {
        let cfg = AppConfig::default();

        let now = Utc::now();

        let below_threshold = consumer_snapshot(89, 10.0, None, None);

        let at_threshold = consumer_snapshot(90, 10.0, None, None);

        assert!(!evaluate_alerts(&below_threshold, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_near_limit"));

        assert!(evaluate_alerts(&at_threshold, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_near_limit"));
    }

    #[test]

    fn consumer_week_near_limit_alert_fires_at_threshold() {
        let cfg = AppConfig::default();

        let now = Utc::now();

        let below_threshold = consumer_snapshot(10, 79.0, None, None);

        let at_threshold = consumer_snapshot(10, 80.0, None, None);

        assert!(!evaluate_alerts(&below_threshold, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_week_near_limit"));

        assert!(evaluate_alerts(&at_threshold, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_week_near_limit"));
    }

    #[test]

    fn consumer_local_snapshots_use_quota_alert_rules() {
        let cfg = AppConfig::default();

        let now = Utc::now();

        let mut snapshot = consumer_snapshot(95, 85.0, None, None);

        snapshot.source = CONSUMER_LOCAL_SOURCE.into();

        snapshot.account_label = "Codex Plus".into();

        let alerts = evaluate_alerts(&snapshot, now, &cfg);

        assert!(alerts
            .iter()
            .any(|alert| alert.code == "quota_5h_near_limit"));

        assert!(alerts
            .iter()
            .any(|alert| alert.code == "quota_week_near_limit"));
    }

    #[test]

    fn consumer_five_hour_use_before_reset_requires_time_and_usage_thresholds() {
        let cfg = AppConfig::default();

        let now = Utc::now();

        let within_window = (now + Duration::minutes(45)).to_rfc3339();

        let outside_window = (now + Duration::minutes(46)).to_rfc3339();

        let within_threshold = consumer_snapshot(20, 10.0, Some(&within_window), None);

        let too_late = consumer_snapshot(20, 10.0, Some(&outside_window), None);

        let too_used = consumer_snapshot(21, 10.0, Some(&within_window), None);

        assert!(evaluate_alerts(&within_threshold, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_unused_before_reset"));

        assert!(!evaluate_alerts(&too_late, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_unused_before_reset"));

        assert!(!evaluate_alerts(&too_used, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_unused_before_reset"));
    }

    #[test]

    fn consumer_week_use_before_reset_requires_time_and_usage_thresholds() {
        let cfg = AppConfig::default();

        let now = Utc::now();

        let within_window = (now + Duration::hours(24)).to_rfc3339();

        let outside_window = (now + Duration::hours(25)).to_rfc3339();

        let within_threshold = consumer_snapshot(10, 40.0, None, Some(&within_window));

        let too_late = consumer_snapshot(10, 40.0, None, Some(&outside_window));

        let too_used = consumer_snapshot(10, 41.0, None, Some(&within_window));

        assert!(evaluate_alerts(&within_threshold, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_week_unused_before_reset"));

        assert!(!evaluate_alerts(&too_late, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_week_unused_before_reset"));

        assert!(!evaluate_alerts(&too_used, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_week_unused_before_reset"));
    }

    #[test]

    fn consumer_use_before_reset_skips_missing_or_invalid_reset_times() {
        let cfg = AppConfig::default();

        let now = Utc::now();

        let missing_reset = consumer_snapshot(5, 10.0, None, None);

        let invalid_reset = consumer_snapshot(5, 10.0, Some("not-a-timestamp"), None);

        assert!(!evaluate_alerts(&missing_reset, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_unused_before_reset"));

        assert!(!evaluate_alerts(&invalid_reset, now, &cfg)
            .iter()
            .any(|alert| alert.code == "quota_5h_unused_before_reset"));
    }

    #[test]

    fn openai_5h_consumer_alerts_can_be_disabled_independently() {
        let cfg = AppConfig {
            openai_consumer_5h_alerts_enabled: false,
            ..AppConfig::default()
        };

        let now = Utc::now();

        let snapshot = consumer_snapshot(95, 85.0, None, None);

        let alerts = evaluate_alerts(&snapshot, now, &cfg);

        assert!(!alerts
            .iter()
            .any(|alert| alert.code.starts_with("quota_5h_")));

        assert!(alerts
            .iter()
            .any(|alert| alert.code.starts_with("quota_week_")));
    }

    #[test]

    fn anthropic_week_consumer_alerts_can_be_disabled_independently() {
        let cfg = AppConfig {
            anthropic_consumer_week_alerts_enabled: false,
            ..AppConfig::default()
        };

        let now = Utc::now();

        let mut snapshot = consumer_snapshot(95, 85.0, None, None);

        snapshot.provider = "anthropic".into();

        snapshot.account_label = "Claude Pro".into();

        let alerts = evaluate_alerts(&snapshot, now, &cfg);

        assert!(alerts
            .iter()
            .any(|alert| alert.code.starts_with("quota_5h_")));

        assert!(!alerts
            .iter()
            .any(|alert| alert.code.starts_with("quota_week_")));
    }

    #[test]

    fn exhausted_alerts_bypass_quiet_hours_but_non_critical_alerts_do_not() {
        let now = Local::now();

        let current_hour = now.hour() as u8;

        let cfg = AppConfig {
            quiet_hours: QuietHours {
                enabled: true,
                start_hour: current_hour,
                end_hour: (current_hour + 1) % 24,
            },
            ..AppConfig::default()
        };

        let critical = Alert {
            level: "critical".into(),
            code: "quota_5h_exhausted".into(),
            message: "critical".into(),
        };

        let warning = Alert {
            level: "warning".into(),
            code: "quota_5h_near_limit".into(),
            message: "warning".into(),
        };

        let info = Alert {
            level: "info".into(),
            code: "quota_week_unused_before_reset".into(),
            message: "info".into(),
        };

        assert!(should_notify_alert(&critical, now, &cfg));

        assert!(!should_notify_alert(&warning, now, &cfg));

        assert!(!should_notify_alert(&info, now, &cfg));

        assert!(should_notify(&[critical], now, &cfg));

        assert!(!should_notify(&[warning, info], now, &cfg));
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

    fn load_config_migrates_legacy_consumer_alert_toggle_to_window_toggles() {
        with_test_config_dir("legacy_oauth_alert_toggle", || {
            let path = config_path().unwrap();

            fs::create_dir_all(path.parent().unwrap()).unwrap();

            fs::write(
                &path,
                serde_json::json!({
                    "near_limit_ratio": 0.85,
                    "inactive_threshold_hours": 8,
                    "quiet_hours": {
                        "enabled": true,
                        "start_hour": 23,
                        "end_hour": 8
                    },
                    "refresh_interval_secs": 15,
                    "api": {},
                    "openai_oauth_alerts_enabled": false
                })
                .to_string(),
            )
            .unwrap();

            let cfg = load_config().unwrap();

            assert!(!cfg.openai_consumer_5h_alerts_enabled);

            assert!(!cfg.openai_consumer_week_alerts_enabled);

            let saved = fs::read_to_string(&path).unwrap();

            let saved: Value = serde_json::from_str(&saved).unwrap();

            assert!(saved.get("openai_oauth_alerts_enabled").is_none());

            assert_eq!(
                saved
                    .get("openai_consumer_5h_alerts_enabled")
                    .and_then(|value| value.as_bool()),
                Some(false)
            );

            assert_eq!(
                saved
                    .get("openai_consumer_week_alerts_enabled")
                    .and_then(|value| value.as_bool()),
                Some(false)
            );
        });
    }

    #[test]

    fn parse_claude_local_usage_response_normalizes_fraction_utilization() {
        let value: Value = serde_json::json!({
            "five_hour": {
                "utilization": 0.62,
                "resets_at": 1773709200i64
            }
        });

        let (window, secondary) =
            parse_claude_local_usage_response(&value).expect("windows missing");

        assert_eq!(window.used_percent, Some(62.0));

        assert!(window.reset_at.is_some());

        assert!(secondary.is_none());
    }

    #[test]

    fn parse_claude_local_usage_response_accepts_percent_utilization() {
        let value: Value = serde_json::json!({
            "five_hour": {
                "utilization": 73.0
            }
        });

        let (window, secondary) =
            parse_claude_local_usage_response(&value).expect("windows missing");

        assert_eq!(window.used_percent, Some(73.0));

        assert!(secondary.is_none());
    }

    #[test]

    fn parse_codex_local_usage_payload_extracts_quota_windows() {
        let payload: Value = serde_json::json!({
            "type": "token_count",
            "rate_limits": {
                "plan_type": "plus",
                "primary": {
                    "used_percent": 18.0,
                    "resets_at": 1773706757i64
                },
                "secondary": {
                    "used_percent": 68.0,
                    "resets_at": 1773855809i64
                }
            }
        });

        let snapshot = parse_codex_local_usage_payload(&payload).unwrap();

        assert_eq!(snapshot.provider, "openai");

        assert_eq!(snapshot.account_label, "Codex Plus");

        assert_eq!(snapshot.source, CONSUMER_LOCAL_SOURCE);

        assert_eq!(snapshot.tokens_in, 18);

        assert!((snapshot.spent_usd - 68.0).abs() < f64::EPSILON);

        assert_eq!(snapshot.limit_usd, 100.0);

        assert!(snapshot.primary_reset_at.is_some());

        assert!(snapshot.secondary_reset_at.is_some());
    }

    #[test]

    fn openai_consumer_usage_skips_null_rate_limits_entries() {
        with_codex_consumer_override(
            "skip_null_rate_limits",
            r#"{"tokens":{"access_token":"present"}}"#,
            &[
                &serde_json::json!({
                    "timestamp": "2026-03-17T15:21:39Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "rate_limits": {
                            "limit_id": "codex",
                            "plan_type": "plus",
                            "primary": {
                                "used_percent": 18.0,
                                "resets_at": 1773706757i64
                            },
                            "secondary": {
                                "used_percent": 68.0,
                                "resets_at": 1773855809i64
                            }
                        }
                    }
                })
                .to_string(),
                &serde_json::json!({
                    "timestamp": "2026-03-17T16:02:15Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {
                            "total_token_usage": {
                                "input_tokens": 1
                            }
                        },
                        "rate_limits": Value::Null
                    }
                })
                .to_string(),
            ],
            || {
                let snapshot = fetch_openai_consumer_usage().expect("snapshot missing");

                assert_eq!(snapshot.source, CONSUMER_LOCAL_SOURCE);

                assert_eq!(snapshot.account_label, "Codex Plus");

                assert_eq!(snapshot.tokens_in, 18);

                assert!((snapshot.spent_usd - 68.0).abs() < f64::EPSILON);
            },
        );
    }

    #[test]

    fn openai_consumer_usage_prefers_global_codex_limit_over_model_specific_limit() {
        with_codex_consumer_override(
            "prefer_global_limit",
            r#"{"tokens":{"access_token":"present"}}"#,
            &[
                &serde_json::json!({
                    "timestamp": "2026-03-17T15:21:42Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "rate_limits": {
                            "limit_id": "codex",
                            "plan_type": "plus",
                            "primary": {
                                "used_percent": 10.0,
                                "resets_at": 1773762721i64
                            },
                            "secondary": {
                                "used_percent": 86.0,
                                "resets_at": 1773855809i64
                            }
                        }
                    }
                })
                .to_string(),
                &serde_json::json!({
                    "timestamp": "2026-03-17T15:21:43Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "rate_limits": {
                            "limit_id": "codex_bengalfox",
                            "limit_name": "GPT-5.3-Codex-Spark",
                            "primary": {
                                "used_percent": 1.0,
                                "resets_at": 1773762721i64
                            },
                            "secondary": {
                                "used_percent": 3.0,
                                "resets_at": 1773855809i64
                            }
                        }
                    }
                })
                .to_string(),
            ],
            || {
                let snapshot = fetch_openai_consumer_usage().expect("snapshot missing");

                assert_eq!(snapshot.source, CONSUMER_LOCAL_SOURCE);

                assert_eq!(snapshot.tokens_in, 10);

                assert!((snapshot.spent_usd - 86.0).abs() < f64::EPSILON);
            },
        );
    }

    #[test]

    fn openai_consumer_usage_falls_back_to_auth_json_usage_when_sessions_missing() {
        let usage_response = serde_json::json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 24.0,
                    "reset_at": 1773188548i64
                },
                "secondary_window": {
                    "used_percent": 61.0,
                    "reset_at": 1773757308i64
                }
            }
        })
        .to_string();

        with_codex_local_usage_override(
            "auth_json_fallback",
            r#"{"tokens":{"access_token":"present","account_id":"acct_123"}}"#,
            &[],
            Some(&usage_response),
            || {
                let snapshot = fetch_openai_consumer_usage().expect("snapshot missing");

                assert_eq!(snapshot.provider, "openai");
                assert_eq!(snapshot.source, CONSUMER_LOCAL_SOURCE);
                assert_eq!(snapshot.account_label, "Codex Pro");
                assert_eq!(snapshot.tokens_in, 24);
                assert!((snapshot.spent_usd - 61.0).abs() < f64::EPSILON);
                assert!(has_openai_consumer_usage());
            },
        );
    }

    #[test]

    fn openai_consumer_usage_falls_back_to_status_snapshot_without_sessions() {
        with_codex_consumer_override(
            "status_only",
            r#"{"tokens":{"access_token":"present"}}"#,
            &[],
            || {
                let snapshot = fetch_openai_consumer_usage().unwrap();

                assert_eq!(snapshot.provider, "openai");

                assert_eq!(snapshot.source, CONSUMER_LOCAL_STATUS_SOURCE);

                assert_eq!(
                    snapshot.status_code.as_deref(),
                    Some("consumer_local_waiting_for_usage")
                );
            },
        );
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
                let plan_type = get_anthropic_consumer_plan_type();

                assert_eq!(plan_type.as_deref(), Some("Pro"));
            },
        );
    }

    #[test]

    fn anthropic_consumer_usage_uses_local_status_snapshot() {
        with_claude_credentials_override(
            "consumer_status",
            r#"{
                "claudeAiOauth": {
                    "subscriptionType": "max",
                    "rateLimitTier": "default_claude_ai"
                }
            }"#,
            || {
                let snapshot = fetch_anthropic_consumer_usage().unwrap();

                assert_eq!(snapshot.provider, "anthropic");

                assert_eq!(snapshot.account_label, "Claude Code Max");

                assert_eq!(snapshot.source, CONSUMER_LOCAL_STATUS_SOURCE);

                assert_eq!(
                    snapshot.status_code.as_deref(),
                    Some("consumer_local_usage_pending")
                );
            },
        );
    }

    #[test]

    fn claude_consumer_usage_shows_pending_when_no_quota_available() {
        with_claude_local_override(
            "pending",
            r#"{
                "claudeAiOauth": {
                    "subscriptionType": "pro",
                    "rateLimitTier": "default_claude_ai",
                    "accessToken": "present",
                    "expiresAt": 4102444800000
                }
            }"#,
            None,
            || {
                let snapshot = fetch_anthropic_consumer_usage().unwrap();

                assert_eq!(snapshot.source, CONSUMER_LOCAL_STATUS_SOURCE);

                assert_eq!(
                    snapshot.status_code.as_deref(),
                    Some("consumer_local_usage_pending")
                );
            },
        );
    }

    #[test]

    fn claude_consumer_usage_fetches_local_token_usage_windows() {
        let usage_response = serde_json::json!({
            "five_hour": {
                "utilization": 62.0,
                "resets_at": "2026-03-19T10:00:00Z"
            },
            "seven_day": {
                "utilization": 74.0,
                "resets_at": "2026-03-23T10:00:00Z"
            }
        })
        .to_string();

        with_claude_local_override(
            "local_usage_api",
            r#"{
                "claudeAiOauth": {
                    "subscriptionType": "pro",
                    "rateLimitTier": "default_claude_ai",
                    "accessToken": "present",
                    "expiresAt": 4102444800000
                }
            }"#,
            Some(&usage_response),
            || {
                let snapshot = fetch_anthropic_consumer_usage().unwrap();

                assert_eq!(snapshot.provider, "anthropic");

                assert_eq!(snapshot.source, CONSUMER_LOCAL_SOURCE);

                assert_eq!(
                    snapshot.status_code.as_deref(),
                    Some("consumer_local_quota")
                );

                let consumer_quota = snapshot
                    .consumer_quota
                    .clone()
                    .expect("consumer quota missing");

                let primary = consumer_quota.primary.expect("primary window missing");

                assert_eq!(primary.used_percent, Some(62.0));

                assert_eq!(primary.reset_at.as_deref(), Some("2026-03-19T10:00:00Z"));

                let secondary = consumer_quota.secondary.expect("secondary window missing");

                assert_eq!(secondary.used_percent, Some(74.0));

                assert!(snapshot.api_metrics.is_none());
            },
        );
    }
}









