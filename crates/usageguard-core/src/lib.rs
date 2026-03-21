mod secret_store;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Local, Timelike, Utc};
use secret_store::app_config_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

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
const CLAUDE_LOCAL_USAGE_CACHE_TTL_SECS: i64 = 180;
const CODEX_WHAM_CACHE_TTL_SECS: i64 = 60;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 4;
const HTTP_REQUEST_TIMEOUT_SECS: u64 = 12;
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
const STANDARD_NEAR_LIMIT_RATIO: f64 = 0.85;
const STANDARD_INACTIVE_THRESHOLD_HOURS: u32 = 8;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub quiet_hours: QuietHours,
    #[serde(default = "default_refresh_interval_secs")]
    pub refresh_interval_secs: u32,
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
    #[serde(
        default = "default_consumer_alerts_enabled",
        alias = "openai_oauth_5h_alerts_enabled"
    )]
    pub openai_consumer_5h_alerts_enabled: bool,
    /// Whether Codex weekly consumer alerts are enabled.
    #[serde(
        default = "default_consumer_alerts_enabled",
        alias = "openai_oauth_week_alerts_enabled"
    )]
    pub openai_consumer_week_alerts_enabled: bool,
    /// Whether Claude Code 5h consumer alerts are enabled.
    #[serde(
        default = "default_consumer_alerts_enabled",
        alias = "anthropic_oauth_5h_alerts_enabled"
    )]
    pub anthropic_consumer_5h_alerts_enabled: bool,
    /// Whether Claude Code weekly consumer alerts are enabled.
    #[serde(
        default = "default_consumer_alerts_enabled",
        alias = "anthropic_oauth_week_alerts_enabled"
    )]
    pub anthropic_consumer_week_alerts_enabled: bool,
    /// Last release tag that already triggered an update notification.
    #[serde(default)]
    pub last_update_notified_version: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            quiet_hours: QuietHours::default(),
            refresh_interval_secs: DEFAULT_REFRESH_INTERVAL_SECS,
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

#[derive(Debug, Clone)]
struct ClaudeAccessTokenState {
    access_token: Option<String>,
    expires_at_ms: Option<i64>,
    expired: bool,
}

fn normalize_epoch_millis(value: i64) -> i64 {
    // Some local credential files may store epoch seconds instead of milliseconds.
    if (1_000_000_000..10_000_000_000).contains(&value) {
        value * 1000
    } else {
        value
    }
}

fn claude_access_token_state() -> Option<ClaudeAccessTokenState> {
    let path = claude_credentials_path()?;

    let raw = fs::read_to_string(path).ok()?;

    let credentials = serde_json::from_str::<ClaudeDesktopCredentials>(&raw).ok()?;

    let oauth = credentials.claude_ai_oauth;

    let access_token = oauth.access_token.filter(|token| !token.trim().is_empty());
    let expires_at_ms = oauth.expires_at_ms.map(normalize_epoch_millis);
    let expired = expires_at_ms.is_some_and(|expires_at_ms| {
        let now_ms = Utc::now().timestamp_millis();
        expires_at_ms < now_ms + 60_000
    });

    Some(ClaudeAccessTokenState {
        access_token,
        expires_at_ms,
        expired,
    })
}

fn fetch_claude_code_usage_from_api(
    access_token: &str,
) -> Option<(ConsumerQuotaWindow, Option<ConsumerQuotaWindow>)> {
    let value = match fetch_claude_local_usage_value(access_token) {
        Ok(value) => value,
        Err(error) => {
            let _ = error;
            return None;
        }
    };

    match parse_claude_local_usage_response(&value) {
        Ok(windows) => Some(windows),
        Err(error) => {
            let _ = error;
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

    let token = claude_access_token_state().and_then(|state| state.access_token);

    let (primary_window, secondary_window) =
        match token.as_deref().and_then(fetch_claude_code_usage_from_api) {
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
    if !has_anthropic_consumer_source() {
        return false;
    }

    claude_insights_cache()
        .lock()
        .unwrap()
        .secondary_window
        .is_some()
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
            let _ = error;
            return None;
        }
    };

    let usage = match parse_openai_local_usage_data(&value) {
        Ok(usage) => usage,
        Err(error) => {
            let _ = error;
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
                    let _ = error;
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

    let (status_code, status_message): (&str, String) = match claude_access_token_state() {
        Some(state) if state.access_token.is_none() => (
            "consumer_local_waiting_for_usage",
            "Claude Code local sign-in detected, but no access token is available yet. Run a Claude command once to refresh local auth.".to_string(),
        ),
        Some(state) if state.expired => {
            let message = state
                .expires_at_ms
                .and_then(DateTime::<Utc>::from_timestamp_millis)
                .map(|timestamp| {
                    format!(
                        "Claude Code local token expired at {}. Run a Claude command once to refresh local auth.",
                        timestamp.to_rfc3339()
                    )
                })
                .unwrap_or_else(|| {
                    "Claude Code local token appears expired. Run a Claude command once to refresh local auth.".to_string()
                });
            ("consumer_local_waiting_for_usage", message)
        }
        _ => (
            "consumer_local_usage_pending",
            "Fetching Claude Code 5h quota...".to_string(),
        ),
    };

    Some(error_snapshot(
        "anthropic",
        &match get_anthropic_consumer_plan_type() {
            Some(plan_type) if is_non_empty(&plan_type) => format!("Claude Code {plan_type}"),
            _ => "Claude Code".to_string(),
        },
        CONSUMER_LOCAL_STATUS_SOURCE,
        Some(status_code),
        Some(&status_message),
    ))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_config_dir()?.join("config.json"))
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

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;

    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read config file: {}", path.display()))?;

    let raw_value = serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("Invalid config JSON: {}", path.display()))?;

    let mut cfg = serde_json::from_value::<AppConfig>(raw_value.clone())
        .with_context(|| format!("Invalid config JSON: {}", path.display()))?;

    let mut migrated = false;

    migrated |= migrate_legacy_consumer_alert_preferences(&raw_value, &mut cfg);

    if [
        "near_limit_ratio",
        "inactive_threshold_hours",
        "api",
        "provider_accounts",
        "profiles",
    ]
    .iter()
    .any(|key| raw_value.get(*key).is_some())
    {
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

fn evaluate_standard_alerts(snapshot: &UsageSnapshot) -> Vec<Alert> {
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
    } else if snapshot.limit_usd > 0.0 && ratio >= STANDARD_NEAR_LIMIT_RATIO {
        alerts.push(Alert {
            level: "warning".into(),
            code: "near_limit".into(),
            message: format!(
                "Near budget limit: ${:.2} / ${:.2}",
                snapshot.spent_usd, snapshot.limit_usd
            ),
        });
    }

    if snapshot.inactive_hours >= STANDARD_INACTIVE_THRESHOLD_HOURS {
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
        evaluate_standard_alerts(snapshot)
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

pub fn provider_snapshots(cfg: &AppConfig) -> Vec<UsageSnapshot> {
    let openai_consumer_label = cfg
        .openai_consumer_label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);

    let anthropic_consumer_label = cfg
        .anthropic_consumer_label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);

    std::thread::scope(|scope| {
        let openai_handle = scope.spawn(move || {
            let mut snapshot = fetch_openai_consumer_usage()?;

            if let Some(label) = openai_consumer_label {
                snapshot.account_label = label;
            }

            Some(snapshot)
        });

        let anthropic_handle = scope.spawn(move || {
            let mut snapshot = fetch_anthropic_consumer_usage()?;

            if let Some(label) = anthropic_consumer_label {
                snapshot.account_label = label;
            }

            Some(snapshot)
        });

        let mut items = Vec::new();

        if let Some(snapshot) = openai_handle.join().ok().flatten() {
            items.push(snapshot);
        }

        if let Some(snapshot) = anthropic_handle.join().ok().flatten() {
            items.push(snapshot);
        }

        items
    })
}

fn client_with_timeout() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .timeout(std::time::Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(Into::into)
}

fn fetch_openai_local_usage_value(access_token: &str, account_id: &str) -> Result<Value> {
    #[cfg(test)]
    if let Some(raw) = openai_local_usage_response_override() {
        return serde_json::from_str(&raw).map_err(Into::into);
    }

    let client = client_with_timeout()?;

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
        let body = resp.text().unwrap_or_default();
        return Err(anyhow!(
            "Codex local usage endpoint returned HTTP {status}: {}",
            body.trim()
        ));
    }

    resp.json().map_err(Into::into)
}

fn fetch_claude_local_usage_value(access_token: &str) -> Result<Value> {
    #[cfg(test)]
    if let Some(raw) = claude_local_usage_response_override() {
        return serde_json::from_str(&raw).map_err(Into::into);
    }

    let client = client_with_timeout()?;

    let resp = client
        .get(CLAUDE_CODE_USAGE_API_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("anthropic-beta", ANTHROPIC_LOCAL_USAGE_BETA_HEADER)
        .header("User-Agent", "usageguard/0.1")
        .send()?;

    let status = resp.status();

    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(anyhow!(
            "Claude local usage endpoint returned HTTP {status}: {}",
            body.trim()
        ));
    }

    resp.json().map_err(Into::into)
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

fn pick_str<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()))
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

        let result = catch_unwind(AssertUnwindSafe(test));

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

        let result = catch_unwind(AssertUnwindSafe(test));

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

        let result = catch_unwind(AssertUnwindSafe(test));

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

        let result = catch_unwind(AssertUnwindSafe(test));

        invalidate_codex_wham_cache();
        invalidate_claude_local_insights_cache();

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

    fn load_config_strips_legacy_api_fields() {
        with_test_config_dir("legacy_api_fields", || {
            let path = config_path().unwrap();

            fs::create_dir_all(path.parent().unwrap()).unwrap();

            fs::write(
                &path,
                serde_json::json!({
                    "near_limit_ratio": 0.85,
                    "inactive_threshold_hours": 24,
                    "quiet_hours": {
                        "enabled": true,
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
                    ],
                    "profiles": [
                        {
                            "id": "openai-default",
                            "label": "OpenAI",
                            "endpoint": "https://api.openai.com/v1/organization/costs"
                        }
                    ]
                })
                .to_string(),
            )
            .unwrap();

            let cfg = load_config().unwrap();

            assert_eq!(cfg.refresh_interval_secs, DEFAULT_REFRESH_INTERVAL_SECS);

            let saved = fs::read_to_string(&path).unwrap();

            let saved: Value = serde_json::from_str(&saved).unwrap();

            assert!(saved.get("near_limit_ratio").is_none());

            assert!(saved.get("inactive_threshold_hours").is_none());

            assert!(saved.get("api").is_none());

            assert!(saved.get("provider_accounts").is_none());

            assert!(saved.get("profiles").is_none());

            assert!(saved.get("quiet_hours").is_some());
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
                    Some("consumer_local_waiting_for_usage")
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

    #[test]

    fn claude_consumer_usage_fetches_windows_even_if_local_expiry_is_stale() {
        let usage_response = serde_json::json!({
            "five_hour": {
                "utilization": 58.0,
                "resets_at": "2026-03-19T10:00:00Z"
            },
            "seven_day": {
                "utilization": 71.0,
                "resets_at": "2026-03-23T10:00:00Z"
            }
        })
        .to_string();

        with_claude_local_override(
            "stale_local_expiry",
            r#"{
                "claudeAiOauth": {
                    "subscriptionType": "pro",
                    "rateLimitTier": "default_claude_ai",
                    "accessToken": "present",
                    "expiresAt": 1700000000000
                }
            }"#,
            Some(&usage_response),
            || {
                let snapshot = fetch_anthropic_consumer_usage().unwrap();

                assert_eq!(snapshot.source, CONSUMER_LOCAL_SOURCE);

                let consumer_quota = snapshot
                    .consumer_quota
                    .clone()
                    .expect("consumer quota missing");

                let primary = consumer_quota.primary.expect("primary window missing");

                assert_eq!(primary.used_percent, Some(58.0));

                let secondary = consumer_quota.secondary.expect("secondary window missing");

                assert_eq!(secondary.used_percent, Some(71.0));
            },
        );
    }
}
