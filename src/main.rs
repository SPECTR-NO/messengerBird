//! MessengerBird — straight SA828 radio sender (SiloMonitor radio path, no vision).
//! Multiple independent sender endpoints share one radio channel.

mod cloud;
mod config;
mod http;
mod netinfo;
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
const CONTENT_W: f32 = 420.0;
const HISTORY_W: f32 = 260.0;
const HISTORY_H: f32 = 320.0;

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn main() -> eframe::Result<()> {
    let cfg = Config::load();
    apply_radio_runtime(&cfg);
    cloud::init();
    cloud::spawn_startup_sync(cfg.clone());

    if let Err(e) = http::spawn_all(&cfg.senders, &cfg.auth_token) {
        eprintln!("http: {e}");
    }

    let icon = load_window_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([CONTENT_W + HISTORY_W + 56.0, 620.0])
        .with_min_inner_size([620.0, 460.0])
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
    remote_host: String,
    host_source: &'static str,
    copied_flash: Option<(String, std::time::Instant)>,
    cloud_email: String,
    cloud_password: String,
    cloud_note: String,
    cloud_note_ok: bool,
    show_cloud_sign_in: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        configure_style(&cc.egui_ctx);
        let st = cloud::status();
        Self {
            cfg,
            note: String::new(),
            note_ok: true,
            logo: load_logo_texture(&cc.egui_ctx),
            remote_host: netinfo::visionlab_host(),
            host_source: netinfo::host_source_label(),
            copied_flash: None,
            cloud_email: st.email,
            cloud_password: String::new(),
            cloud_note: String::new(),
            cloud_note_ok: true,
            show_cloud_sign_in: st.configured && !st.signed_in,
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
        let meta = radio::TxMeta {
            sender_id: sender.id.clone(),
            source: "ui",
            client_host: None,
        };
        match radio::transmit_with(&phrase, meta) {
            Ok(()) => self.set_note(format!("Transmitting “{phrase}”…"), true),
            Err(e) => self.set_note(e, false),
        }
    }

    fn cloud_sign_in(&mut self) {
        match cloud::sign_in(&self.cloud_email, &self.cloud_password) {
            Ok(()) => {
                self.cloud_password.clear();
                self.cloud_note = "Signed in".into();
                self.cloud_note_ok = true;
                self.show_cloud_sign_in = false;
                cloud::spawn_startup_sync(self.cfg.clone());
            }
            Err(e) => {
                self.cloud_note = e;
                self.cloud_note_ok = false;
            }
        }
    }

    fn cloud_sign_up(&mut self) {
        match cloud::sign_up(&self.cloud_email, &self.cloud_password) {
            Ok(()) => {
                self.cloud_password.clear();
                self.cloud_note = "Account ready — signed in".into();
                self.cloud_note_ok = true;
                self.show_cloud_sign_in = false;
                cloud::spawn_startup_sync(self.cfg.clone());
            }
            Err(e) => {
                self.cloud_note = e;
                self.cloud_note_ok = false;
            }
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
    // Clear blue mark when dragging to select text.
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(45, 130, 217, 110);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, BRAND);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, STROKE);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.interaction.selectable_labels = true;
    style.interaction.multi_widget_text_select = true;
    ctx.set_style(style);
}

/// Selectable label — drag to mark, Ctrl/Cmd+C to copy.
fn mark(ui: &mut egui::Ui, text: RichText) -> egui::Response {
    ui.add(egui::Label::new(text).selectable(true))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(status) = radio::take_status() {
            let ok = radio::status_ok(&status);
            self.set_note(status, ok);
        }
        cloud::tick_heartbeat(&self.cfg);
        // Keep text marking on every frame (some egui paths reset interaction flags).
        ctx.style_mut(|style| {
            style.interaction.selectable_labels = true;
            style.interaction.multi_widget_text_select = true;
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("app_scroll")
                    .auto_shrink([false, false])
                    .drag_to_scroll(false)
                    .show(ui, |ui| {
                        self.header_bar(ui);
                        ui.add_space(10.0);

                        ui.vertical_centered(|ui| {
                            let row_w = CONTENT_W + 12.0 + HISTORY_W;
                            ui.set_width(row_w.min(ui.available_width()));

                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(CONTENT_W);
                                    self.status_row(ui);
                                    ui.add_space(8.0);
                                    self.cloud_card(ui);
                                    ui.add_space(8.0);
                                    self.remote_card(ui);
                                    ui.add_space(8.0);
                                    self.radio_card(ui);
                                    ui.add_space(10.0);
                                    section_label(ui, "Senders");
                                    ui.add_space(4.0);
                                    self.senders(ui);
                                    ui.add_space(8.0);
                                });

                                ui.add_space(12.0);

                                ui.vertical(|ui| {
                                    ui.set_width(HISTORY_W);
                                    self.history_card(ui);
                                });
                            });
                        });
                    });
            });
    }
}

impl App {
    fn header_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(
                egui::Image::new((self.logo.id(), Vec2::splat(28.0)))
                    .fit_to_exact_size(Vec2::splat(28.0)),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("MessengerBird")
                    .size(18.0)
                    .strong()
                    .color(TEXT),
            );
            ui.label(RichText::new("· plant radio").size(12.0).color(MUTED));
        });
    }

    fn status_row(&self, ui: &mut egui::Ui) {
        let (label, color) = match radio::line_lamp() {
            LineLamp::Tx => ("Transmitting", RED),
            LineLamp::Busy => ("Channel busy", AMBER),
            LineLamp::Off if self.cfg.radio.muted => ("Muted", MUTED),
            LineLamp::Off => ("Idle", GREEN),
        };
        let last = radio::last_message()
            .map(|m| format!("Last · {m}"))
            .unwrap_or_else(|| "Last · none".into());

        card(ui, |ui| {
            ui.horizontal(|ui| {
                status_pill(ui, label, color);
                ui.add_space(10.0);
                ui.label(
                    RichText::new(peltor::channel_label(self.cfg.radio.channel))
                        .size(13.0)
                        .color(TEXT),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(last).size(12.0).color(MUTED));
                });
            });
        });
    }

    fn cloud_card(&mut self, ui: &mut egui::Ui) {
        let st = cloud::status();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Cloud").size(11.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if st.configured && st.signed_in {
                        if link_btn(ui, "Sync").clicked() {
                            cloud::spawn_startup_sync(self.cfg.clone());
                        }
                        if link_btn(ui, "Sign out").clicked() {
                            cloud::sign_out();
                            self.show_cloud_sign_in = true;
                            self.cloud_note.clear();
                        }
                    } else if st.configured {
                        if link_btn(ui, if self.show_cloud_sign_in { "Hide" } else { "Sign in" })
                            .clicked()
                        {
                            self.show_cloud_sign_in = !self.show_cloud_sign_in;
                        }
                    }
                });
            });

            ui.add_space(4.0);
            let (pill, color) = if !st.configured {
                ("Offline · no config", MUTED)
            } else if st.signed_in {
                ("Signed in", GREEN)
            } else {
                ("Signed out", AMBER)
            };
            ui.horizontal(|ui| {
                status_pill(ui, pill, color);
                if st.signed_in && !st.email.is_empty() {
                    ui.label(RichText::new(&st.email).size(12.0).color(TEXT));
                }
            });

            if let Some(ts) = st.last_sync {
                ui.label(
                    RichText::new(format!("Last sync · {}", format_when(ts)))
                        .size(11.0)
                        .color(MUTED),
                );
            }
            if let Some(err) = &st.last_error {
                ui.label(RichText::new(err).size(11.0).color(RED));
            }
            if !self.cloud_note.is_empty() {
                ui.label(
                    RichText::new(&self.cloud_note)
                        .size(11.0)
                        .color(if self.cloud_note_ok { GREEN } else { RED }),
                );
            }

            if st.configured && (!st.signed_in || self.show_cloud_sign_in) && !st.signed_in {
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.cloud_email)
                        .hint_text("email")
                        .desired_width(ui.available_width()),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.cloud_password)
                        .password(true)
                        .hint_text("password")
                        .desired_width(ui.available_width()),
                );
                ui.horizontal(|ui| {
                    if primary_btn(ui, "Sign in").clicked() {
                        self.cloud_sign_in();
                    }
                    if ghost_btn(ui, "Create").clicked() {
                        self.cloud_sign_up();
                    }
                });
                ui.label(
                    RichText::new("Plant cloud only — VisionLab operators don’t need this.")
                        .size(11.0)
                        .color(MUTED),
                );
            } else if !st.configured {
                ui.label(
                    RichText::new("Copy config/supabase.example.yaml → supabase.yaml")
                        .size(11.0)
                        .color(MUTED),
                );
            }
        });
    }

    fn remote_card(&mut self, ui: &mut egui::Ui) {
        if let Some((_, at)) = &self.copied_flash {
            if at.elapsed().as_secs() >= 2 {
                self.copied_flash = None;
            }
        }

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("VisionLab host · {}", self.host_source))
                        .size(11.0)
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if link_btn(ui, "Refresh").clicked() {
                        self.remote_host = netinfo::visionlab_host();
                        self.host_source = netinfo::host_source_label();
                    }
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                mark(
                    ui,
                    RichText::new(&self.remote_host)
                        .size(14.0)
                        .strong()
                        .color(TEXT),
                );
                if ghost_btn(ui, "Copy").clicked() {
                    ui.ctx().copy_text(self.remote_host.clone());
                    self.copied_flash =
                        Some(("Host copied".into(), std::time::Instant::now()));
                }
            });

            ui.add_space(6.0);
            let host = self.remote_host.clone();
            let rows: Vec<(String, u16, String)> = self
                .cfg
                .senders
                .iter()
                .map(|s| {
                    (
                        s.display_name().to_string(),
                        s.port(),
                        s.remote_tx_url(&host),
                    )
                })
                .collect();
            let mut copied_url = false;
            for (name, port, url) in &rows {
                ui.horizontal(|ui| {
                    mark(
                        ui,
                        RichText::new(format!("{name}  :{port}"))
                            .size(12.0)
                            .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if link_btn(ui, "Copy URL").clicked() {
                            ui.ctx().copy_text(url.clone());
                            copied_url = true;
                        }
                    });
                });
                mark(
                    ui,
                    RichText::new(url)
                        .size(11.0)
                        .color(Color32::from_rgb(110, 110, 118)),
                );
            }
            if copied_url {
                self.copied_flash = Some(("URL copied".into(), std::time::Instant::now()));
            }

            ui.add_space(4.0);
            ui.label(
                RichText::new("Off-LAN: Tailscale, or forward 8787–8789.")
                    .size(11.0)
                    .color(MUTED),
            );
            if !self.cfg.auth_token.trim().is_empty() {
                ui.label(
                    RichText::new("Auth on — send X-MessengerBird-Token.")
                        .size(11.0)
                        .color(AMBER),
                );
            }
            if let Some((msg, _)) = &self.copied_flash {
                ui.label(RichText::new(msg).size(11.0).color(GREEN));
            }
        });
    }

    fn radio_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Channel").size(11.0).color(MUTED));
                egui::ComboBox::from_id_salt("channel")
                    .selected_text(format!("Ch {}", self.cfg.radio.channel))
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for ch in 1u8..=16 {
                            ui.selectable_value(
                                &mut self.cfg.radio.channel,
                                ch,
                                peltor::channel_label(ch),
                            );
                        }
                    });
                let mut muted = self.cfg.radio.muted;
                if ui.checkbox(&mut muted, "Mute").changed() {
                    self.cfg.radio.muted = muted;
                    radio::set_muted(muted);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_btn(ui, "Read").clicked() {
                        self.read_module();
                    }
                    if ghost_btn(ui, "Program").clicked() {
                        self.program_module();
                    }
                    if ghost_btn(ui, "Save").clicked() {
                        self.save_and_apply();
                    }
                });
            });
            if !self.note.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&self.note)
                        .size(11.0)
                        .color(if self.note_ok { GREEN } else { RED }),
                );
            }
        });
    }

    fn history_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.set_min_height(HISTORY_H);
            ui.set_max_height(HISTORY_H);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Alert history").size(11.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if link_btn(ui, "Clear").clicked() {
                        radio::clear_history();
                    }
                });
            });
            ui.add_space(6.0);

            let events = radio::history();
            if events.is_empty() {
                ui.label(
                    RichText::new("No alerts yet.")
                        .size(12.0)
                        .color(MUTED),
                );
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt("history_list")
                .max_height(HISTORY_H - 40.0)
                .auto_shrink([false, false])
                .drag_to_scroll(false)
                .show(ui, |ui| {
                    for (i, event) in events.iter().enumerate() {
                        if i > 0 {
                            ui.add_space(6.0);
                        }
                        let status_color = if event.ok { GREEN } else { RED };
                        let status = if event.ok { "Sent" } else { "Failed" };
                        ui.horizontal(|ui| {
                            mark(
                                ui,
                                RichText::new(&event.message)
                                    .size(12.0)
                                    .strong()
                                    .color(TEXT),
                            );
                            ui.label(
                                RichText::new(status)
                                    .size(11.0)
                                    .strong()
                                    .color(status_color),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    mark(
                                        ui,
                                        RichText::new(format_when(event.at))
                                            .size(11.0)
                                            .color(MUTED),
                                    );
                                },
                            );
                        });
                    }
                });
        });
    }

    fn senders(&mut self, ui: &mut egui::Ui) {
        let host = self.remote_host.clone();
        let count = self.cfg.senders.len();
        for idx in 0..count {
            let (name, port, sender_muted, url) = {
                let s = &self.cfg.senders[idx];
                (
                    s.display_name().to_string(),
                    s.port(),
                    s.muted,
                    s.remote_tx_url(&host),
                )
            };

            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&name).size(14.0).strong().color(TEXT));
                        mark(
                            ui,
                            RichText::new(format!(":{port}  {url}"))
                                .size(11.0)
                                .color(Color32::from_rgb(110, 110, 118)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tx_enabled =
                            !self.cfg.radio.muted && !sender_muted && !radio::busy();
                        ui.add_enabled_ui(tx_enabled, |ui| {
                            if primary_btn(ui, "TX").clicked() {
                                self.transmit_sender(idx);
                            }
                        });
                        let mut muted = sender_muted;
                        if ui.checkbox(&mut muted, "Mute").changed() {
                            self.cfg.senders[idx].muted = muted;
                        }
                    });
                });
            });
            ui.add_space(6.0);
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
        egui::FontId::proportional(12.0),
        color,
    );
    let pad = Vec2::new(10.0, 4.0);
    let size = galley.size() + pad * 2.0;
    let (rect, _resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(12),
        color.linear_multiply(0.16),
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        color,
    );
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0_f32, STROKE))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn ghost_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(TEXT))
            .fill(BTN)
            .stroke(egui::Stroke::new(1.0_f32, STROKE))
            .corner_radius(7.0)
            .min_size(egui::vec2(58.0, 26.0)),
    )
}

fn link_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(11.0).color(MUTED))
            .fill(Color32::TRANSPARENT)
            .frame(false),
    )
}

fn primary_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(12.0).strong().color(Color32::WHITE))
            .fill(BRAND)
            .corner_radius(7.0)
            .min_size(egui::vec2(52.0, 28.0)),
    )
}
