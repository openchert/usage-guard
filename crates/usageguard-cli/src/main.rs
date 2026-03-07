use clap::{Parser, Subcommand};
use usageguard_core::{demo_snapshots, evaluate_alerts, AppConfig, UsageSnapshot};

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
}

fn print_snapshot(s: &UsageSnapshot, cfg: &AppConfig) {
    println!("Provider: {} ({})", s.provider, s.account_label);
    println!("Spend: ${:.2} / ${:.2}", s.spent_usd, s.limit_usd);
    println!("Tokens: in={} out={}", s.tokens_in, s.tokens_out);
    println!("Inactive: {}h", s.inactive_hours);
    println!("Source: {}", s.source);
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

fn main() {
    let cli = Cli::parse();
    let cfg = AppConfig::default();

    match cli.command {
        Command::Demo => {
            for s in demo_snapshots() {
                print_snapshot(&s, &cfg);
            }
        }
        Command::Check {
            spent,
            limit,
            inactive_hours,
        } => {
            let s = UsageSnapshot {
                provider: "custom".into(),
                account_label: "local".into(),
                spent_usd: spent,
                limit_usd: limit,
                tokens_in: 0,
                tokens_out: 0,
                inactive_hours,
                source: "cli".into(),
            };
            print_snapshot(&s, &cfg);
        }
    }
}
