use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub near_limit_ratio: f64,
    pub inactive_threshold_hours: u32,
    pub quiet_hours: QuietHours,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            near_limit_ratio: 0.85,
            inactive_threshold_hours: 8,
            quiet_hours: QuietHours::default(),
        }
    }
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

trait Hour {
    fn hour(&self) -> u32;
}

impl Hour for DateTime<Local> {
    fn hour(&self) -> u32 {
        use chrono::Timelike;
        Timelike::hour(self)
    }
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
}
