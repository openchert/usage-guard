use chrono::Local;
use eframe::egui;
use usageguard_core::{
    evaluate_alerts, load_config, provider_snapshots, save_config, should_notify, AppConfig,
    UsageSnapshot,
};

struct UsageGuardApp {
    cfg: AppConfig,
    snapshots: Vec<UsageSnapshot>,
    show_connect: bool,
    openai_key_input: String,
    anthropic_key_input: String,
    openai_endpoint_input: String,
    anthropic_endpoint_input: String,
    status: String,
    last_updated: String,
}

impl Default for UsageGuardApp {
    fn default() -> Self {
        let cfg = load_config().unwrap_or_default();
        let mut app = Self {
            openai_key_input: cfg.api.openai_api_key.clone().unwrap_or_default(),
            anthropic_key_input: cfg.api.anthropic_api_key.clone().unwrap_or_default(),
            openai_endpoint_input: cfg.api.openai_costs_endpoint.clone().unwrap_or_default(),
            anthropic_endpoint_input: cfg.api.anthropic_costs_endpoint.clone().unwrap_or_default(),
            cfg,
            snapshots: Vec::new(),
            show_connect: false,
            status: String::new(),
            last_updated: "never".to_string(),
        };
        app.refresh();
        app
    }
}

impl UsageGuardApp {
    fn refresh(&mut self) {
        self.snapshots = provider_snapshots(&self.cfg);
        self.last_updated = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    }

    fn validate_inputs(&self) -> Result<(), String> {
        if !self.openai_endpoint_input.trim().is_empty()
            && !self.openai_endpoint_input.starts_with("https://")
        {
            return Err("OpenAI endpoint must start with https://".to_string());
        }
        if !self.anthropic_endpoint_input.trim().is_empty()
            && !self.anthropic_endpoint_input.starts_with("https://")
        {
            return Err("Anthropic endpoint must start with https://".to_string());
        }
        Ok(())
    }
}

impl eframe::App for UsageGuardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            ui.heading("UsageGuard");
            ui.label("Calm local usage monitor");

            ui.horizontal(|ui| {
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }
                ui.small(format!("Last updated: {}", self.last_updated));
            });

            for s in &self.snapshots {
                let alerts = evaluate_alerts(s, &self.cfg);
                let percent = if s.limit_usd > 0.0 {
                    (s.spent_usd / s.limit_usd).clamp(0.0, 1.2)
                } else {
                    0.0
                };

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.strong(&s.provider);
                        ui.label(format!("${:.2}/${:.2}", s.spent_usd, s.limit_usd));
                        if s.source.starts_with("api-error:") {
                            ui.colored_label(egui::Color32::from_rgb(220, 130, 80), "API error");
                        }
                    });
                    ui.add(egui::ProgressBar::new(percent as f32).show_percentage());
                    ui.label(format!("inactive {}h", s.inactive_hours));
                    ui.small(format!("source: {}", s.source));

                    if alerts.is_empty() {
                        ui.label("status: ok");
                    } else {
                        for a in &alerts {
                            ui.label(format!("{}: {}", a.level, a.message));
                        }
                    }

                    let _ = should_notify(&alerts, Local::now(), &self.cfg);
                });
            }

            ui.separator();
            if ui.button("Connect API").clicked() {
                self.show_connect = !self.show_connect;
            }

            if self.show_connect {
                ui.group(|ui| {
                    ui.label("OpenAI API key");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.openai_key_input)
                            .password(true)
                            .hint_text("sk-..."),
                    );

                    ui.label("OpenAI costs endpoint (optional)");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.openai_endpoint_input)
                            .hint_text("https://api.openai.com/v1/organization/costs"),
                    );

                    ui.label("Anthropic API key");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.anthropic_key_input)
                            .password(true)
                            .hint_text("sk-ant-..."),
                    );

                    ui.label("Anthropic usage endpoint (optional)");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.anthropic_endpoint_input)
                            .hint_text("https://api.anthropic.com/v1/organizations/usage"),
                    );

                    if ui.button("Save locally").clicked() {
                        match self.validate_inputs() {
                            Ok(_) => {
                                self.cfg.api.openai_api_key =
                                    if self.openai_key_input.trim().is_empty() {
                                        None
                                    } else {
                                        Some(self.openai_key_input.trim().to_string())
                                    };
                                self.cfg.api.anthropic_api_key =
                                    if self.anthropic_key_input.trim().is_empty() {
                                        None
                                    } else {
                                        Some(self.anthropic_key_input.trim().to_string())
                                    };
                                self.cfg.api.openai_costs_endpoint =
                                    if self.openai_endpoint_input.trim().is_empty() {
                                        None
                                    } else {
                                        Some(self.openai_endpoint_input.trim().to_string())
                                    };
                                self.cfg.api.anthropic_costs_endpoint =
                                    if self.anthropic_endpoint_input.trim().is_empty() {
                                        None
                                    } else {
                                        Some(self.anthropic_endpoint_input.trim().to_string())
                                    };

                                match save_config(&self.cfg) {
                                    Ok(_) => {
                                        self.status = "Saved local config.".to_string();
                                        self.refresh();
                                    }
                                    Err(e) => self.status = format!("Save failed: {e}"),
                                }
                            }
                            Err(msg) => self.status = msg,
                        }
                    }
                });
            }

            if !self.status.is_empty() {
                ui.small(&self.status);
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size([420.0, 560.0])
            .with_transparent(false),
        ..Default::default()
    };

    eframe::run_native(
        "UsageGuard",
        options,
        Box::new(|_cc| Ok(Box::<UsageGuardApp>::default())),
    )
}
