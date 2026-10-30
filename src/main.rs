//! MessengerBird — straight SA828 radio sender (SiloMonitor radio path, no vision).

mod config;
mod http;
mod peltor;
mod radio;
mod sa828;

use config::Config;
use eframe::egui::{self, Color32, RichText};
use radio::LineLamp;

const BG: Color32 = Color32::from_rgb(14, 14, 16);
const CARD: Color32 = Color32::from_rgb(28, 28, 30);
const TEXT: Color32 = Color32::from_rgb(245, 245, 247);
const MUTED: Color32 = Color32::from_rgb(142, 142, 147);
const GREEN: Color32 = Color32::from_rgb(48, 209, 88);
const RED: Color32 = Color32::from_rgb(255, 69, 58);
const AMBER: Color32 = Color32::from_rgb(255, 159, 10);
const BTN: Color32 = Color32::from_rgb(44, 44, 46);

fn main() -> eframe::Result<()> {
    let cfg = Config::load();
    apply_radio_runtime(&cfg);

    if let Err(e) = http::spawn(&cfg.listen) {
        eprintln!("http: {e}");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 520.0])
            .with_min_inner_size([360.0, 420.0])
            .with_title("MessengerBird"),
        ..Default::default()
    };

    eframe::run_native(
        "MessengerBird",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(cfg)))),
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
    test_message: String,
}

impl App {
    fn new(cfg: Config) -> Self {
        Self {
            cfg,
            note: String::new(),
            note_ok: true,
            test_message: "Alert".into(),
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

    fn transmit(&mut self) {
        match radio::transmit_message(&self.test_message) {
            Ok(()) => self.set_note("Transmitting…", true),
            Err(e) => self.set_note(e, false),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(status) = radio::take_status() {
            let ok = radio::status_ok(&status);
            self.set_note(status, ok);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = BG;
        visuals.override_text_color = Some(TEXT);
        ctx.set_visuals(visuals);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);
            ui.label(RichText::new("MessengerBird").size(26.0).strong().color(TEXT));
            ui.label(
                RichText::new("Straight radio sender · SA828 UHF")
                    .size(13.0)
                    .color(MUTED),
            );
            ui.add_space(18.0);

            card(ui, |ui| {
                let (label, color) = match radio::line_lamp() {
                    LineLamp::Tx => ("Transmitting", RED),
                    LineLamp::Busy => ("Channel busy", AMBER),
                    LineLamp::Off if self.cfg.radio.muted => ("Muted", MUTED),
                    LineLamp::Off => ("Idle", GREEN),
                };
                ui.label(RichText::new(label).size(28.0).strong().color(color));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(peltor::channel_label(self.cfg.radio.channel))
                        .size(14.0)
                        .color(TEXT),
                );
                if let Some(last) = radio::last_message() {
                    ui.label(RichText::new(format!("Last · {last}")).size(13.0).color(MUTED));
                } else {
                    ui.label(RichText::new("Last · none").size(13.0).color(MUTED));
                }
                ui.label(
                    RichText::new(format!("VisionLab → http://{}/tx", self.cfg.listen))
                        .size(12.0)
                        .color(MUTED),
                );
            });

            ui.add_space(12.0);

            card(ui, |ui| {
                ui.label(RichText::new("Channel").size(12.0).color(MUTED));
                egui::ComboBox::from_id_salt("channel")
                    .selected_text(format!("Ch {}", self.cfg.radio.channel))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for ch in 1u8..=16 {
                            ui.selectable_value(
                                &mut self.cfg.radio.channel,
                                ch,
                                peltor::channel_label(ch),
                            );
                        }
                    });

                ui.add_space(10.0);
                ui.label(RichText::new("Test message").size(12.0).color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut self.test_message)
                        .desired_width(280.0)
                        .text_color(TEXT),
                );

                ui.add_space(10.0);
                let mut muted = self.cfg.radio.muted;
                if ui.checkbox(&mut muted, "Mute").changed() {
                    self.cfg.radio.muted = muted;
                    radio::set_muted(muted);
                }

                ui.add_space(14.0);
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
                    let tx_enabled = !self.cfg.radio.muted && !radio::busy();
                    ui.add_enabled_ui(tx_enabled, |ui| {
                        if primary_btn(ui, "Transmit").clicked() {
                            self.transmit();
                        }
                    });
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
        });
    }
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, add);
}

fn ghost_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(TEXT))
            .fill(BTN)
            .corner_radius(8.0)
            .min_size(egui::vec2(72.0, 34.0)),
    )
}

fn primary_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(Color32::from_rgb(10, 132, 255))
            .corner_radius(8.0)
            .min_size(egui::vec2(100.0, 34.0)),
    )
}
