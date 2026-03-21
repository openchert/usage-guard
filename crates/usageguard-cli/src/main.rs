use chrono::Utc;
use clap::{Parser, Subcommand};
use usageguard_core::{evaluate_alerts, load_config, provider_snapshots, AppConfig, UsageSnapshot};

#[derive(Parser)]
#[command(name = "usageguard", about = "UsageGuard CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(alias = "demo")]
    Status,
}

fn print_snapshot(snapshot: &UsageSnapshot, cfg: &AppConfig) {
    println!(
        "Provider: {} ({})",
        snapshot.provider, snapshot.account_label
    );
    println!("Source: {}", snapshot.source);

    if let Some(quota) = &snapshot.consumer_quota {
        if let Some(primary) = &quota.primary {
            if let Some(used) = primary.used_percent {
                println!("5h used: {:.0}%", used);
            }
            if let Some(reset_at) = &primary.reset_at {
                println!("5h resets: {}", reset_at);
            }
        }

        if let Some(secondary) = &quota.secondary {
            if let Some(used) = secondary.used_percent {
                println!("Week used: {:.0}%", used);
            }
            if let Some(reset_at) = &secondary.reset_at {
                println!("Week resets: {}", reset_at);
            }
        }
    }

    if let Some(message) = &snapshot.status_message {
        println!("Status: {}", message);
    }

    let alerts = evaluate_alerts(snapshot, Utc::now(), cfg);
    if alerts.is_empty() {
        println!("Alerts: none\n");
    } else {
        for alert in alerts {
            println!("- [{}] {}", alert.level, alert.message);
        }
        println!();
    }
}

fn load_config_or_exit() -> AppConfig {
    match load_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            println!("{error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let cfg = load_config_or_exit();

    match cli.command {
        Command::Status => {
            for snapshot in provider_snapshots(&cfg) {
                print_snapshot(&snapshot, &cfg);
            }
        }
    }
}
