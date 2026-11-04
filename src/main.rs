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
    let mut cfg = Config::load();
    cfg.ensure_persisted_secrets();
    apply_radio_runtime(&cfg);
    radio::load_history();
    cloud::init();
    cloud::spawn_startup_sync(cfg.clone());

    if let Err(e) = http::spawn_all(&cfg.senders, &cfg.auth_token) {
        eprintln!("http: {e}");
    }
    http::publish_runtime(&cfg.senders, cfg.rate_per_minute, &cfg.allow_cidrs);

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
    radio::set_dry_run(cfg.radio.dry_run);
    radio::set_csma(cfg.radio.channel_wait_secs, cfg.radio.force_tx_if_busy);
    let _ = radio::select_channel(cfg.radio.channel);
    http::set_auth_token(&cfg.auth_token);
    http::publish_runtime(&cfg.senders, cfg.rate_per_minute, &cfg.allow_cidrs);
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
    show_token: bool,
    show_about: bool,
    show_wizard: bool,
    reveal_token: bool,
    new_sender_name: String,
    allow_cidr_edit: String,
    host_ticks: u32,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        configure_style(&cc.egui_ctx);
        let st = cloud::status();
        let allow = cfg.allow_cidrs.join(", ");
        let minted = cfg.token_just_minted;
        let wizard = !cfg.wizard_done;
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
            show_token: minted,
            show_about: false,
            show_wizard: wizard,
            reveal_token: minted,
            new_sender_name: String::new(),
            allow_cidr_edit: allow,
            host_ticks: 0,
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
            Ok(radio::TxAccept::Started) => self.set_note(format!("Transmitting “{phrase}”…"), true),
            Ok(radio::TxAccept::Queued) => self.set_note(format!("Queued “{phrase}”"), true),
            Err(e) => self.set_note(e, false),
        }
    }

    fn persist(&mut self) {
        apply_radio_runtime(&self.cfg);
        if let Err(e) = self.cfg.save() {
            self.set_note(format!("Save failed: {e}"), false);
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
                self.cloud_note = if e.to_ascii_lowercase().contains("confirm") {
                    format!("{e} Open the email, confirm, then sign in here.")
                } else {
                    e
                };
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
        self.host_ticks = self.host_ticks.wrapping_add(1);
        if self.host_ticks.is_multiple_of(75) {
            self.remote_host = netinfo::visionlab_host();
            self.host_source = netinfo::host_source_label();
        }
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
                                    self.auth_banner(ui);
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
        if self.show_about {
            self.about_window(ctx);
        }
        if self.show_wizard {
            self.wizard_window(ctx);
        }
    }
}

impl App {
    fn header_bar(&mut self, ui: &mut egui::Ui) {
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if link_btn(ui, "About").clicked() {
                    self.show_about = true;
                }
            });
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
                    } else if st.configured
                        && link_btn(ui, if self.show_cloud_sign_in { "Hide" } else { "Sign in" })
                            .clicked()
                    {
                        self.show_cloud_sign_in = !self.show_cloud_sign_in;
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
                    if ghost_btn(ui, "Reset").clicked() {
                        match cloud::request_password_reset(&self.cloud_email) {
                            Ok(()) => {
                                self.cloud_note = "Password reset email sent. Confirm the account, then sign in.".into();
                                self.cloud_note_ok = true;
                            }
                            Err(e) => {
                                self.cloud_note = e;
                                self.cloud_note_ok = false;
                            }
                        }
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
            if !self.cfg.auth_enabled || self.cfg.auth_token.trim().is_empty() {
                ui.label(
                    RichText::new("Open TX — anyone who can reach these ports can transmit.")
                        .size(11.0)
                        .color(RED),
                );
            } else if self.show_token || self.reveal_token {
                ui.label(
                    RichText::new("New token — copy it into VisionLab, then hide it.")
                        .size(11.0)
                        .color(AMBER),
                );
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.cfg.auth_token)
                        .hint_text("auth token")
                        .desired_width(ui.available_width() - 8.0)
                        .password(!self.reveal_token),
                );
            });
            ui.horizontal(|ui| {
                if link_btn(ui, if self.reveal_token { "Hide" } else { "Show" }).clicked() {
                    self.reveal_token = !self.reveal_token;
                    self.show_token = false;
                }
                if link_btn(ui, "Copy token").clicked() {
                    ui.ctx().copy_text(self.cfg.auth_token.clone());
                    self.copied_flash = Some(("Token copied".into(), std::time::Instant::now()));
                }
                if link_btn(ui, "Rotate").clicked() {
                    self.cfg.auth_token = config::random_token();
                    self.cfg.auth_enabled = true;
                    self.reveal_token = true;
                    self.persist();
                    self.set_note("Token rotated", true);
                }
                if link_btn(ui, if self.cfg.auth_enabled { "Disable" } else { "Enable" }).clicked()
                {
                    self.cfg.auth_enabled = !self.cfg.auth_enabled;
                    if self.cfg.auth_enabled && self.cfg.auth_token.is_empty() {
                        self.cfg.auth_token = config::random_token();
                        self.reveal_token = true;
                    }
                    self.persist();
                }
            });
            ui.label(RichText::new("Allow CIDRs (comma, empty = any)").size(11.0).color(MUTED));
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.allow_cidr_edit)
                        .hint_text("10.0.0.0/8, 100.64.0.0/10")
                        .desired_width(ui.available_width()),
                )
                .lost_focus()
            {
                self.cfg.allow_cidrs = self
                    .allow_cidr_edit
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                self.persist();
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
                    self.persist();
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
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Squelch").size(11.0).color(MUTED));
                let mut sq = self.cfg.radio.squelch as f32;
                if ui.add(egui::Slider::new(&mut sq, 0.0..=8.0).integer()).changed() {
                    self.cfg.radio.squelch = sq as u8;
                }
                ui.label(RichText::new("CTCSS").size(11.0).color(MUTED));
                egui::ComboBox::from_id_salt("ctcss")
                    .selected_text(peltor::ctcss_label(self.cfg.radio.ctcss))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cfg.radio.ctcss, 0, "Off");
                        for idx in 1u8..=38 {
                            ui.selectable_value(
                                &mut self.cfg.radio.ctcss,
                                idx,
                                peltor::ctcss_label(idx),
                            );
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Audio").size(11.0).color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut self.cfg.radio.audio_device)
                        .desired_width(ui.available_width()),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("CSMA wait s").size(11.0).color(MUTED));
                let mut wait = self.cfg.radio.channel_wait_secs as f32;
                if ui
                    .add(egui::Slider::new(&mut wait, 1.0..=120.0).integer())
                    .changed()
                {
                    self.cfg.radio.channel_wait_secs = wait as u64;
                    radio::set_csma(
                        self.cfg.radio.channel_wait_secs,
                        self.cfg.radio.force_tx_if_busy,
                    );
                }
                let mut force = self.cfg.radio.force_tx_if_busy;
                if ui.checkbox(&mut force, "TX if still busy").changed() {
                    self.cfg.radio.force_tx_if_busy = force;
                    radio::set_csma(self.cfg.radio.channel_wait_secs, force);
                }
                let mut dry = self.cfg.radio.dry_run;
                if ui.checkbox(&mut dry, "Dry run").changed() {
                    self.cfg.radio.dry_run = dry;
                    radio::set_dry_run(dry);
                }
            });
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
                        ui.horizontal(|ui| {
                            let status_color = if event.ok { GREEN } else { RED };
                            let status = if event.ok { "Sent" } else { "Failed" };
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
                        let meta = format!(
                            "{} · {}{}",
                            if event.source.is_empty() {
                                "—"
                            } else {
                                event.source.as_str()
                            },
                            if event.sender_id.is_empty() {
                                "—"
                            } else {
                                event.sender_id.as_str()
                            },
                            if event.client.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", event.client)
                            }
                        );
                        ui.label(RichText::new(meta).size(10.0).color(MUTED));
                        if !event.detail.is_empty() {
                            ui.label(RichText::new(&event.detail).size(10.0).color(MUTED));
                        }
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
                            self.persist();
                        }
                        if link_btn(ui, "Remove").clicked() {
                            self.cfg.senders.remove(idx);
                            self.persist();
                            self.set_note("Sender removed — restart to free its port", true);
                        }
                    });
                });
                if self.cfg.senders.get(idx).is_none() {
                    return;
                }
                let mut phrase = self.cfg.senders[idx].message.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Phrase").size(11.0).color(MUTED));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut phrase)
                                .desired_width(ui.available_width()),
                        )
                        .lost_focus()
                        && phrase.trim() != self.cfg.senders[idx].message
                    {
                        self.cfg.senders[idx].message = phrase.trim().to_string();
                        self.cfg.senders[idx].name = self.cfg.senders[idx].message.clone();
                        self.persist();
                        self.set_note("Phrase saved — same port, no restart", true);
                    }
                });
            });
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_sender_name)
                    .hint_text("New phrase")
                    .desired_width(180.0),
            );
            if ghost_btn(ui, "Add sender").clicked() && !self.new_sender_name.trim().is_empty() {
                let n = self.cfg.senders.len() as u16;
                let phrase = self.new_sender_name.trim().to_string();
                let id = phrase
                    .to_ascii_lowercase()
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect::<String>();
                self.cfg.senders.push(config::SenderCfg {
                    id: if id.is_empty() {
                        format!("sender_{}", n + 1)
                    } else {
                        id
                    },
                    name: phrase.clone(),
                    listen: format!("0.0.0.0:{}", 8787 + n),
                    message: phrase,
                    muted: false,
                });
                self.new_sender_name.clear();
                self.persist();
                self.set_note("Sender added — restart MessengerBird to bind the new port", true);
            }
        });
    }

    fn auth_banner(&mut self, ui: &mut egui::Ui) {
        if self.cfg.auth_enabled && !self.cfg.auth_token.is_empty() {
            return;
        }
        card(ui, |ui| {
            ui.label(
                RichText::new("TX is unauthenticated on 0.0.0.0. Enable a token before leaving the lab.")
                    .size(12.0)
                    .color(RED),
            );
        });
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new("About MessengerBird")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("VisionLab ↔ SA828 plant radio sender.");
                ui.label("PTT is open-drain (LOW = TX, idle = High-Z).");
                ui.label("Prefer Tailscale. Do not expose 8787–8789 on the public internet without the token.");
                ui.label("HTTPS: put a reverse proxy in front; this app speaks plain HTTP.");
                if ui.button("Hardware checklist").clicked() {
                    self.show_wizard = true;
                }
            });
        self.show_about = open;
    }

    fn wizard_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_wizard;
        let mut done = false;
        egui::Window::new("First-run checklist")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("1. UART: Pi 5 needs dtoverlay=uart0-pi5, then reboot.");
                ui.label("2. Wiring: PTT BCM23, SPKEN BCM24, channel bits 5/6/12/13, audio to MIC.");
                ui.label("3. Press Read, then Program, confirm channel 3.");
                ui.label("4. TX each sender once and listen on the headset.");
                ui.label("5. Copy the auth token into VisionLab.");
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        if done {
            self.cfg.wizard_done = true;
            self.persist();
            open = false;
        }
        self.show_wizard = open;
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
