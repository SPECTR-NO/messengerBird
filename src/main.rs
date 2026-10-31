//! MessengerBird — straight SA828 radio sender (SiloMonitor radio path, no vision).
//! Multiple independent sender endpoints share one radio channel.

mod config;
mod http;
mod peltor;
mod radio;
mod sa828;

use config::Config;
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2};
use radio::LineLamp;

/// Brand blue from the MessengerBird mark.
const BRAND: Color32 = Color32::from_rgb(45, 130, 217);
const BG: Color32 = Color32::from_rgb(10, 10, 12);
const CARD: Color32 = Color32::from_rgb(22, 22, 24);
const STROKE: Color32 = Color32::from_rgb(40, 40, 44);
const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
const MUTED: Color32 = Color32::from_rgb(142, 142, 147);
const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
const RED: Color32 = Color32::from_rgb(255, 69, 58);
const AMBER: Color32 = Color32::from_rgb(255, 159, 10);
const BTN: Color32 = Color32::from_rgb(36, 36, 40);
const CONTENT_W: f32 = 400.0;
const HISTORY_W: f32 = 280.0;
const HISTORY_H: f32 = 280.0;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn main() -> eframe::Result<()> {
    let cfg = Config::load();
    apply_radio_runtime(&cfg);

    if let Err(e) = http::spawn_all(&cfg.senders) {
        eprintln!("http: {e}");
    }

    let height = 420.0 + (cfg.senders.len() as f32) * 118.0;
    let icon = load_window_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([CONTENT_W + HISTORY_W + 72.0, height.clamp(560.0, 920.0)])
        .with_min_inner_size([640.0, 520.0])
        .with_title("MessengerBird");
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "MessengerBird",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, cfg)))),
    )
}

fn load_window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn load_logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let image = image::load_from_memory(ICON_PNG)
        .expect("embedded icon")
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    ctx.load_texture(
        "messengerbird-logo",
        color,
        egui::TextureOptions::LINEAR,
    )
}

fn apply_radio_runtime(cfg: &Config) {
    radio::set_ptt_pin(cfg.radio.ptt_gpio);
    radio::set_spken_pin(cfg.radio.spken_gpio);
    radio::set_channel_pins(
        cfg.radio.ch_bit1_gpio,
        cfg.radio.ch_bit2_gpio,
        cfg.radio.ch_bit4_gpio,
        cfg.radio.ch_bit8_gpio,
    );
    radio::set_audio_device(&cfg.radio.audio_device);
    radio::set_muted(cfg.radio.muted);
    let _ = radio::select_channel(cfg.radio.channel);
}

struct App {
    cfg: Config,
    note: String,
    note_ok: bool,
    logo: egui::TextureHandle,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        configure_style(&cc.egui_ctx);
        Self {
            cfg,
            note: String::new(),
            note_ok: true,
            logo: load_logo_texture(&cc.egui_ctx),
        }
    }

    fn set_note(&mut self, msg: impl Into<String>, ok: bool) {
        self.note = msg.into();
        self.note_ok = ok;
    }

    fn save_and_apply(&mut self) {
        self.cfg.radio.channel = peltor::clamp_channel(self.cfg.radio.channel);
        self.cfg.radio.ctcss = peltor::clamp_ctcss(self.cfg.radio.ctcss);
        self.cfg.radio.squelch = sa828::clamp_squelch(self.cfg.radio.squelch);
        apply_radio_runtime(&self.cfg);
        match self.cfg.save() {
            Ok(()) => match radio::select_channel(self.cfg.radio.channel) {
                Ok(()) => self.set_note(
                    format!("Saved · {}", peltor::channel_label(self.cfg.radio.channel)),
                    true,
                ),
                Err(e) => self.set_note(format!("Saved, channel pins: {e}"), false),
            },
            Err(e) => self.set_note(format!("Save failed: {e}"), false),
        }
    }

    fn program_module(&mut self) {
        match sa828::program_plan(
            self.cfg.radio.squelch,
            self.cfg.radio.ctcss,
            &self.cfg.radio.uart_port,
        ) {
            Ok(msg) => {
                let pins = radio::select_channel(self.cfg.radio.channel);
                let ok = pins.is_ok();
                let extra = match pins {
                    Ok(()) => format!("\n{}", peltor::channel_label(self.cfg.radio.channel)),
                    Err(e) => format!("\n{e}"),
                };
                self.set_note(format!("{msg}{extra}"), ok);
            }
            Err(e) => self.set_note(format!("Program failed: {e}"), false),
        }
    }

    fn read_module(&mut self) {
        match sa828::read(&self.cfg.radio.uart_port) {
            Ok(msg) => self.set_note(msg, true),
            Err(e) => self.set_note(format!("Read failed: {e}"), false),
        }
    }

    fn transmit_sender(&mut self, idx: usize) {
        let Some(sender) = self.cfg.senders.get(idx) else {
            return;
        };
        if sender.muted {
            self.set_note(format!("{} is muted", sender.display_name()), false);
            return;
        }
        let phrase = sender.phrase().to_string();
        match radio::transmit_message(&phrase) {
            Ok(()) => self.set_note(format!("Transmitting “{phrase}”…"), true),
            Err(e) => self.set_note(e, false),
        }
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.inactive.bg_fill = BTN;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 48, 54);
    visuals.widgets.active.bg_fill = Color32::from_rgb(56, 56, 64);
    visuals.selection.bg_fill = BRAND.linear_multiply(0.35);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, STROKE);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    ctx.set_style(style);
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(status) = radio::take_status() {
            let ok = radio::status_ok(&status);
            self.set_note(status, ok);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(24, 20)),
            )
            .show(ctx, |ui| {
                self.header(ui);
                ui.add_space(24.0);

                // Center the boxes only — logo stays upper-left.
                ui.vertical_centered(|ui| {
                    let row_w = CONTENT_W + 16.0 + HISTORY_W;
                    ui.set_width(row_w.min(ui.available_width()));

                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(CONTENT_W);
                            self.status_card(ui);
                            ui.add_space(12.0);
                            self.radio_card(ui);
                            ui.add_space(18.0);
                            section_label(ui, "Senders");
                            ui.add_space(8.0);
                            self.senders(ui);
                        });

                        ui.add_space(16.0);

                        ui.vertical(|ui| {
                            ui.set_width(HISTORY_W);
                            self.history_card(ui);
                        });
                    });
                });
            });
    }
}

impl App {
    fn header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::Image::new((self.logo.id(), Vec2::splat(40.0)))
                    .fit_to_exact_size(Vec2::splat(40.0)),
            );
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("MessengerBird")
                        .size(22.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("Plant radio sender")
                        .size(12.0)
                        .color(MUTED),
                );
            });
        });
    }

    fn status_card(&self, ui: &mut egui::Ui) {
        let (label, color) = match radio::line_lamp() {
            LineLamp::Tx => ("Transmitting", RED),
            LineLamp::Busy => ("Channel busy", AMBER),
            LineLamp::Off if self.cfg.radio.muted => ("Muted", MUTED),
            LineLamp::Off => ("Idle", GREEN),
        };

        card(ui, |ui| {
            ui.vertical_centered(|ui| {
                status_pill(ui, label, color);
                ui.add_space(12.0);
                ui.label(
                    RichText::new(peltor::channel_label(self.cfg.radio.channel))
                        .size(15.0)
                        .color(TEXT),
                );
                ui.add_space(4.0);
                let last = radio::last_message()
                    .map(|m| format!("Last · {m}"))
                    .unwrap_or_else(|| "Last · none".into());
                ui.label(RichText::new(last).size(12.0).color(MUTED));
            });
        });
    }

    fn radio_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Shared channel").size(11.0).color(MUTED));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("channel")
                    .selected_text(format!("Ch {}", self.cfg.radio.channel))
                    .width(128.0)
                    .show_ui(ui, |ui| {
                        for ch in 1u8..=16 {
                            ui.selectable_value(
                                &mut self.cfg.radio.channel,
                                ch,
                                peltor::channel_label(ch),
                            );
                        }
                    });

                ui.add_space(8.0);
                let mut muted = self.cfg.radio.muted;
                if ui.checkbox(&mut muted, "Mute all").changed() {
                    self.cfg.radio.muted = muted;
                    radio::set_muted(muted);
                }
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ghost_btn(ui, "Save").clicked() {
                    self.save_and_apply();
                }
                if ghost_btn(ui, "Program").clicked() {
                    self.program_module();
                }
                if ghost_btn(ui, "Read").clicked() {
                    self.read_module();
                }
            });

            if !self.note.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(&self.note)
                        .size(12.0)
                        .color(if self.note_ok { GREEN } else { RED }),
                );
            }
        });
    }

    fn history_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.set_max_height(HISTORY_H);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Alert history").size(11.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Clear").size(12.0).color(MUTED))
                                .fill(Color32::TRANSPARENT)
                                .frame(false),
                        )
                        .clicked()
                    {
                        radio::clear_history();
                    }
                });
            });
            ui.add_space(10.0);

            let events = radio::history();
            if events.is_empty() {
                ui.label(
                    RichText::new("No alerts yet.\nTransmits show up here.")
                        .size(13.0)
                        .color(MUTED),
                );
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt("history_list")
                .max_height(HISTORY_H - 48.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, event) in events.iter().enumerate() {
                        if i > 0 {
                            ui.add_space(10.0);
                        }

                        let status_color = if event.ok { GREEN } else { RED };
                        let status = if event.ok { "Sent" } else { "Failed" };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&event.message)
                                    .size(13.0)
                                    .strong()
                                    .color(TEXT),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format_when(event.at))
                                            .size(11.0)
                                            .color(MUTED),
                                    );
                                },
                            );
                        });
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(status)
                                .size(11.0)
                                .strong()
                                .color(status_color),
                        );
                        if !event.detail.is_empty()
                            && event.detail != format!("Played {}", event.message)
                        {
                            ui.label(
                                RichText::new(&event.detail)
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        }
                    }
                });
        });
    }

    fn senders(&mut self, ui: &mut egui::Ui) {
        let count = self.cfg.senders.len();
        for idx in 0..count {
            let (name, listen, phrase, sender_muted) = {
                let s = &self.cfg.senders[idx];
                (
                    s.display_name().to_string(),
                    s.listen.clone(),
                    s.phrase().to_string(),
                    s.muted,
                )
            };

            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&name).size(16.0).strong().color(TEXT));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!("{phrase}"))
                                .size(12.0)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut muted = sender_muted;
                        if ui.checkbox(&mut muted, "Mute").changed() {
                            self.cfg.senders[idx].muted = muted;
                        }
                    });
                });

                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("http://{listen}/tx"))
                        .size(11.0)
                        .color(Color32::from_rgb(110, 110, 118)),
                );

                ui.add_space(10.0);
                let tx_enabled = !self.cfg.radio.muted && !sender_muted && !radio::busy();
                ui.add_enabled_ui(tx_enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 120.0).max(0.0) * 0.5);
                        if primary_btn(ui, "Transmit").clicked() {
                            self.transmit_sender(idx);
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }
    }
}

fn format_when(unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(unix);
    let ago = now.saturating_sub(unix);
    if ago < 5 {
        "just now".into()
    } else if ago < 60 {
        format!("{ago}s ago")
    } else if ago < 3600 {
        format!("{}m ago", ago / 60)
    } else if ago < 86_400 {
        format!("{}h ago", ago / 3600)
    } else {
        format!("{}d ago", ago / 86_400)
    }
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_ascii_uppercase())
            .size(11.0)
            .strong()
            .color(MUTED),
    );
}

fn status_pill(ui: &mut egui::Ui, label: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(15.0),
        color,
    );
    let pad = Vec2::new(16.0, 8.0);
    let size = galley.size() + pad * 2.0;
    let (rect, _resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(20),
        color.linear_multiply(0.16),
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(15.0),
        color,
    );
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0_f32, STROKE))
        .corner_radius(14.0)
        .inner_margin(egui::Margin::symmetric(18, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn ghost_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).color(TEXT))
            .fill(BTN)
            .stroke(egui::Stroke::new(1.0_f32, STROKE))
            .corner_radius(9.0)
            .min_size(egui::vec2(78.0, 34.0)),
    )
}

fn primary_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).strong().color(Color32::WHITE))
            .fill(BRAND)
            .corner_radius(9.0)
            .min_size(egui::vec2(120.0, 36.0)),
    )
}
