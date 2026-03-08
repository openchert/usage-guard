use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "usage-guard";

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
    pub together_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub azure_openai_api_key: Option<String>,
    pub ollama_api_key: Option<String>,

    pub openai_costs_endpoint: Option<String>,
    pub anthropic_costs_endpoint: Option<String>,
    pub gemini_costs_endpoint: Option<String>,
    pub mistral_costs_endpoint: Option<String>,
    pub groq_costs_endpoint: Option<String>,
    pub together_costs_endpoint: Option<String>,
    pub openrouter_costs_endpoint: Option<String>,
    pub azure_openai_costs_endpoint: Option<String>,
    pub ollama_usage_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub id: String,
    pub label: String,
    pub endpoint: String,
    pub auth_header: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub near_limit_ratio: f64,
    pub inactive_threshold_hours: u32,
    pub quiet_hours: QuietHours,
    pub api: ApiCredentials,
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
            profiles: vec![],
        }
    }
}

fn keyring_entry(provider_id: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("provider.{provider_id}.api_key"),
    )?)
}

pub fn set_provider_api_key(provider_id: &str, key: Option<&str>) -> Result<()> {
    let entry = keyring_entry(provider_id)?;
    match key {
        Some(v) if !v.trim().is_empty() => entry.set_password(v.trim())?,
        _ => {
            let _ = entry.delete_credential();
        }
    }
    Ok(())
}

pub fn get_provider_api_key(provider_id: &str) -> Option<String> {
    let entry = keyring_entry(provider_id).ok()?;
    match entry.get_password() {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

pub fn has_provider_api_key(provider_id: &str) -> bool {
    get_provider_api_key(provider_id).is_some()
}

fn resolve_provider_api_key(
    provider_id: &str,
    config_value: Option<String>,
    env_var: &str,
) -> Option<String> {
    get_provider_api_key(provider_id)
        .or(config_value)
        .or_else(|| std::env::var(env_var).ok())
}

struct ProviderSpec<'a> {
    id: &'a str,
    label: &'a str,
    env_prefix: &'a str,
    api_key: Option<String>,
    endpoint: Option<String>,
    default_endpoint: Option<&'a str>,
    auth_header: &'a str,
    extra_headers: Vec<(&'a str, String)>,
    usage_log_env: Option<&'a str>,
}

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("Unable to resolve config directory")?;
    Ok(base.join("usage-guard").join("config.json"))
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

    // migrate plaintext keys to keyring when present
    let mut migrated = false;
    if let Some(v) = cfg.api.openai_api_key.take() {
        let _ = set_provider_api_key("openai", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.anthropic_api_key.take() {
        let _ = set_provider_api_key("anthropic", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.gemini_api_key.take() {
        let _ = set_provider_api_key("gemini", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.mistral_api_key.take() {
        let _ = set_provider_api_key("mistral", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.groq_api_key.take() {
        let _ = set_provider_api_key("groq", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.together_api_key.take() {
        let _ = set_provider_api_key("together", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.openrouter_api_key.take() {
        let _ = set_provider_api_key("openrouter", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.azure_openai_api_key.take() {
        let _ = set_provider_api_key("azure_openai", Some(&v));
        migrated = true;
    }
    if let Some(v) = cfg.api.ollama_api_key.take() {
        let _ = set_provider_api_key("ollama", Some(&v));
        migrated = true;
    }

    if migrated {
        let _ = save_config(&cfg);
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

pub fn provider_snapshots(cfg: &AppConfig) -> Vec<UsageSnapshot> {
    let builtins = vec![
        ProviderSpec {
            id: "openai",
            label: "OpenAI",
            env_prefix: "OPENAI",
            api_key: resolve_provider_api_key(
                "openai",
                cfg.api.openai_api_key.clone(),
                "OPENAI_API_KEY",
            ),
            endpoint: cfg
                .api
                .openai_costs_endpoint
                .clone()
                .or_else(|| std::env::var("OPENAI_COSTS_ENDPOINT").ok()),
            default_endpoint: Some("https://api.openai.com/v1/organization/costs"),
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("OPENAI_USAGE_LOG"),
        },
        ProviderSpec {
            id: "anthropic",
            label: "Anthropic",
            env_prefix: "ANTHROPIC",
            api_key: resolve_provider_api_key(
                "anthropic",
                cfg.api.anthropic_api_key.clone(),
                "ANTHROPIC_API_KEY",
            ),
            endpoint: cfg
                .api
                .anthropic_costs_endpoint
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_COSTS_ENDPOINT").ok()),
            default_endpoint: Some("https://api.anthropic.com/v1/organizations/usage"),
            auth_header: "x-api-key",
            extra_headers: vec![("anthropic-version", "2023-06-01".to_string())],
            usage_log_env: Some("ANTHROPIC_USAGE_LOG"),
        },
        ProviderSpec {
            id: "gemini",
            label: "Gemini",
            env_prefix: "GEMINI",
            api_key: resolve_provider_api_key(
                "gemini",
                cfg.api.gemini_api_key.clone(),
                "GEMINI_API_KEY",
            ),
            endpoint: cfg
                .api
                .gemini_costs_endpoint
                .clone()
                .or_else(|| std::env::var("GEMINI_COSTS_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("GEMINI_USAGE_LOG"),
        },
        ProviderSpec {
            id: "mistral",
            label: "Mistral",
            env_prefix: "MISTRAL",
            api_key: resolve_provider_api_key(
                "mistral",
                cfg.api.mistral_api_key.clone(),
                "MISTRAL_API_KEY",
            ),
            endpoint: cfg
                .api
                .mistral_costs_endpoint
                .clone()
                .or_else(|| std::env::var("MISTRAL_COSTS_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("MISTRAL_USAGE_LOG"),
        },
        ProviderSpec {
            id: "groq",
            label: "Groq",
            env_prefix: "GROQ",
            api_key: resolve_provider_api_key("groq", cfg.api.groq_api_key.clone(), "GROQ_API_KEY"),
            endpoint: cfg
                .api
                .groq_costs_endpoint
                .clone()
                .or_else(|| std::env::var("GROQ_COSTS_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("GROQ_USAGE_LOG"),
        },
        ProviderSpec {
            id: "together",
            label: "Together",
            env_prefix: "TOGETHER",
            api_key: resolve_provider_api_key(
                "together",
                cfg.api.together_api_key.clone(),
                "TOGETHER_API_KEY",
            ),
            endpoint: cfg
                .api
                .together_costs_endpoint
                .clone()
                .or_else(|| std::env::var("TOGETHER_COSTS_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("TOGETHER_USAGE_LOG"),
        },
        ProviderSpec {
            id: "openrouter",
            label: "OpenRouter",
            env_prefix: "OPENROUTER",
            api_key: resolve_provider_api_key(
                "openrouter",
                cfg.api.openrouter_api_key.clone(),
                "OPENROUTER_API_KEY",
            ),
            endpoint: cfg
                .api
                .openrouter_costs_endpoint
                .clone()
                .or_else(|| std::env::var("OPENROUTER_COSTS_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("OPENROUTER_USAGE_LOG"),
        },
        ProviderSpec {
            id: "azure_openai",
            label: "Azure OpenAI",
            env_prefix: "AZURE_OPENAI",
            api_key: resolve_provider_api_key(
                "azure_openai",
                cfg.api.azure_openai_api_key.clone(),
                "AZURE_OPENAI_API_KEY",
            ),
            endpoint: cfg
                .api
                .azure_openai_costs_endpoint
                .clone()
                .or_else(|| std::env::var("AZURE_OPENAI_COSTS_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "api-key",
            extra_headers: vec![],
            usage_log_env: Some("AZURE_OPENAI_USAGE_LOG"),
        },
        ProviderSpec {
            id: "ollama",
            label: "Ollama",
            env_prefix: "OLLAMA",
            api_key: resolve_provider_api_key(
                "ollama",
                cfg.api.ollama_api_key.clone(),
                "OLLAMA_API_KEY",
            ),
            endpoint: cfg
                .api
                .ollama_usage_endpoint
                .clone()
                .or_else(|| std::env::var("OLLAMA_USAGE_ENDPOINT").ok()),
            default_endpoint: None,
            auth_header: "Authorization",
            extra_headers: vec![],
            usage_log_env: Some("OLLAMA_USAGE_LOG"),
        },
    ];

    let mut items: Vec<UsageSnapshot> = builtins
        .into_iter()
        .filter_map(fetch_provider_snapshot)
        .collect();

    for profile in &cfg.profiles {
        let snapshot = fetch_custom_profile(profile);
        if let Some(s) = snapshot {
            items.push(s);
        }
    }

    if items.is_empty() {
        demo_snapshots()
    } else {
        items
    }
}

fn fetch_custom_profile(profile: &ProviderProfile) -> Option<UsageSnapshot> {
    if profile.endpoint.trim().is_empty() {
        return None;
    }

    let mut headers = vec![];
    if let (Some(name), Some(key)) = (&profile.auth_header, &profile.api_key) {
        if name.eq_ignore_ascii_case("authorization") {
            headers.push((name.as_str(), format!("Bearer {key}")));
        } else {
            headers.push((name.as_str(), key.clone()));
        }
    }

    match snapshot_from_http_json(
        &profile.endpoint,
        &headers,
        &profile.id,
        &profile.label,
        "profile-api",
    ) {
        Ok(s) => Some(s),
        Err(e) => Some(error_snapshot(
            &profile.id,
            &profile.label,
            format!("api-error:{e}"),
        )),
    }
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
        let auth_value = if spec.auth_header.eq_ignore_ascii_case("authorization") {
            format!("Bearer {key}")
        } else {
            key
        };

        let mut headers = vec![(spec.auth_header, auth_value)];
        for h in spec.extra_headers {
            headers.push(h);
        }

        match snapshot_from_http_json(&url, &headers, spec.id, spec.label, "api") {
            Ok(s) => return Some(s),
            Err(e) => {
                return Some(error_snapshot(
                    spec.id,
                    spec.label,
                    format!("api-error:{e}"),
                ));
            }
        }
    }

    env_fallback_snapshot(spec.id, spec.label, spec.env_prefix)
}

fn snapshot_from_http_json(
    url: &str,
    headers: &[(&str, String)],
    provider: &str,
    label: &str,
    source: &str,
) -> Result<UsageSnapshot> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;

    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, v);
    }

    let res = req.send()?.error_for_status()?;
    let value: Value = res.json()?;

    // strict-ish known responses first
    if provider == "openai" {
        if let Ok(s) = parse_openai_costs_response(&value) {
            return Ok(s);
        }
    }
    if provider == "anthropic" {
        if let Ok(s) = parse_anthropic_usage_response(&value) {
            return Ok(s);
        }
    }

    snapshot_from_value(&value, provider, label, source)
}

fn parse_openai_costs_response(value: &Value) -> Result<UsageSnapshot> {
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
        account_label: "OpenAI".into(),
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
        source: "openai-api".into(),
    })
}

fn parse_anthropic_usage_response(value: &Value) -> Result<UsageSnapshot> {
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
        account_label: "Anthropic".into(),
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
        source: "anthropic-api".into(),
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
    snapshot_from_value(&value, provider, label, "ndjson")
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
    })
}

fn error_snapshot(provider: &str, label: &str, source: String) -> UsageSnapshot {
    UsageSnapshot {
        provider: provider.to_string(),
        account_label: label.to_string(),
        spent_usd: 0.0,
        limit_usd: 0.0,
        tokens_in: 0,
        tokens_out: 0,
        inactive_hours: 0,
        source,
    }
}

fn pick_f64(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_f64()))
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
}
