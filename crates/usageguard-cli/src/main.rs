use clap::{Parser, Subcommand};
use usageguard_core::{
    evaluate_alerts, has_provider_api_key, load_config, provider_snapshots, save_config,
    set_provider_api_key, AppConfig, UsageSnapshot,
};

#[derive(Parser)]
#[command(name = "usageguard", about = "UsageGuard CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Demo,
    Check {
        #[arg(long)]
        spent: f64,
        #[arg(long)]
        limit: f64,
        #[arg(long, default_value_t = 0)]
        inactive_hours: u32,
    },
    Config {
        #[arg(long)]
        show: bool,
        #[arg(long)]
        openai_key: Option<String>,
        #[arg(long)]
        anthropic_key: Option<String>,
    },
}

fn print_snapshot(s: &UsageSnapshot, cfg: &AppConfig) {
    println!("Provider: {} ({})", s.provider, s.account_label);
    if let Some(metrics) = &s.api_metrics {
        println!(
            "Today: spend=${:.2} tokens(in={}, out={})",
            metrics.today.spend_usd, metrics.today.tokens_in, metrics.today.tokens_out
        );
        println!(
            "30d: spend=${:.2} tokens(in={}, out={})",
            metrics.rolling_30d.spend_usd,
            metrics.rolling_30d.tokens_in,
            metrics.rolling_30d.tokens_out
        );
        if let Some(requests) = metrics.today.requests {
            println!("Today requests: {}", requests);
        }
        if let Some(requests) = metrics.rolling_30d.requests {
            println!("30d requests: {}", requests);
        }
    } else {
        println!("Spend: ${:.2} / ${:.2}", s.spent_usd, s.limit_usd);
        println!("Tokens: in={} out={}", s.tokens_in, s.tokens_out);
    }
    println!("Inactive: {}h", s.inactive_hours);
    println!("Source: {}", s.source);
    if let Some(message) = &s.status_message {
        println!("Status: {}", message);
    }
    let alerts = evaluate_alerts(s, cfg);
    if alerts.is_empty() {
        println!("Alerts: none\n");
    } else {
        for a in alerts {
            println!("- [{}] {}", a.level, a.message);
        }
        println!();
    }
}

fn load_config_or_exit() -> AppConfig {
    match load_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Demo => {
            let cfg = load_config_or_exit();
            for s in provider_snapshots(&cfg) {
                print_snapshot(&s, &cfg);
            }
        }
        Command::Check {
            spent,
            limit,
            inactive_hours,
        } => {
            let cfg = load_config_or_exit();
            let s = UsageSnapshot {
                provider: "custom".into(),
                account_label: "local".into(),
                spent_usd: spent,
                limit_usd: limit,
                tokens_in: 0,
                tokens_out: 0,
                inactive_hours,
                source: "cli".into(),
                status_code: None,
                status_message: None,
                api_metrics: None,
            };
            print_snapshot(&s, &cfg);
        }
        Command::Config {
            show,
            openai_key,
            anthropic_key,
        } => {
            let mut cfg = load_config_or_exit();
            let has_openai_arg = openai_key.is_some();
            let has_anthropic_arg = anthropic_key.is_some();

            if let Some(k) = openai_key {
                if let Err(e) = set_provider_api_key("openai", Some(&k)) {
                    eprintln!("Failed to store OpenAI key in secure storage: {e}");
                    std::process::exit(1);
                }
                cfg.api.openai_api_key = None;
            }
            if let Some(k) = anthropic_key {
                if let Err(e) = set_provider_api_key("anthropic", Some(&k)) {
                    eprintln!("Failed to store Anthropic key in secure storage: {e}");
                    std::process::exit(1);
                }
                cfg.api.anthropic_api_key = None;
            }

            if has_openai_arg || has_anthropic_arg {
                if let Err(e) = save_config(&cfg) {
                    eprintln!("Failed to save config: {e}");
                    std::process::exit(1);
                }
                println!("Config saved.");
            }

            if show || (!has_openai_arg && !has_anthropic_arg) {
                let secure_storage = if cfg!(target_os = "windows") {
                    "windows-dpapi"
                } else {
                    "unsupported"
                };
                println!(
                    "{{\n  \"openai_connected\": {},\n  \"anthropic_connected\": {},\n  \"secure_storage\": \"{}\",\n  \"near_limit_ratio\": {},\n  \"inactive_threshold_hours\": {}\n}}",
                    has_provider_api_key("openai") || cfg.api.openai_api_key.is_some(),
                    has_provider_api_key("anthropic") || cfg.api.anthropic_api_key.is_some(),
                    secure_storage,
                    cfg.near_limit_ratio,
                    cfg.inactive_threshold_hours
                );
            }
        }
    }
}
