use chrono::Local;
use eframe::egui;
use usageguard_core::{demo_snapshots, evaluate_alerts, should_notify, AppConfig};

struct UsageGuardApp {
    cfg: AppConfig,
    connect_clicked: bool,
}

impl Default for UsageGuardApp {
    fn default() -> Self {
        Self {
            cfg: AppConfig::default(),
            connect_clicked: false,
        }
    }
}

impl eframe::App for UsageGuardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
            ui.heading("UsageGuard");
            ui.label("Small local usage monitor");

            for s in demo_snapshots() {
                let alerts = evaluate_alerts(&s, &self.cfg);
                let percent = if s.limit_usd > 0.0 {
                    (s.spent_usd / s.limit_usd).clamp(0.0, 1.2)
                } else {
                    0.0
                };

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(format!("{}", s.provider));
                        ui.label(format!("${:.2}/${:.2}", s.spent_usd, s.limit_usd));
                        ui.label(format!("in:{} out:{}", s.tokens_in, s.tokens_out));
                    });
                    ui.add(egui::ProgressBar::new(percent as f32).show_percentage());
                    ui.label(format!("inactive {}h", s.inactive_hours));
                    if alerts.is_empty() {
                        ui.label("status: ok");
                    } else {
                        for a in &alerts {
                            ui.label(format!("{}: {}", a.level, a.message));
                        }
                    }

                    let _notify = should_notify(&alerts, Local::now(), &self.cfg);
                });
            }

            ui.separator();
            if ui.button("Connect API").clicked() {
                self.connect_clicked = true;
            }
            if self.connect_clicked {
                ui.label("Next: paste API keys in local config (implementation in next step)");
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size([420.0, 520.0])
            .with_transparent(false),
        ..Default::default()
    };

    eframe::run_native(
        "UsageGuard",
        options,
        Box::new(|_cc| Ok(Box::<UsageGuardApp>::default())),
    )
}
