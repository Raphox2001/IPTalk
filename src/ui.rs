//! Das Fenster.

use std::sync::atomic::Ordering::*;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

use crate::audio::Backend;
use crate::config::{Config, Profile};
use crate::dsp::{gain_to_db, TalkMode};
use crate::engine::{AudioEngine, Shared};

const GREEN: Color32 = Color32::from_rgb(80, 200, 120);
const YELLOW: Color32 = Color32::from_rgb(230, 190, 60);
const RED: Color32 = Color32::from_rgb(230, 80, 70);
const GREY: Color32 = Color32::from_rgb(140, 140, 140);

pub struct App {
    cfg: Config,
    shared: Arc<Shared>,
    engine: AudioEngine,
    peer_input: String,
    audio_dirty: bool,
    /// Exklusiv-Modus hat falsch getaktete Daten geliefert → bis zur nächsten Änderung geteilt öffnen.
    force_shared: bool,
    meter_in: f32,
    meter_out: f32,
    message: Option<(String, Color32)>,
}

impl App {
    pub fn new(cfg: Config, shared: Arc<Shared>) -> App {
        let engine = AudioEngine::start(&shared, &cfg, false);
        App {
            peer_input: cfg.last_peer.clone(),
            cfg,
            shared,
            engine,
            audio_dirty: false,
            force_shared: false,
            meter_in: 0.0,
            meter_out: 0.0,
            message: None,
        }
    }

    fn restart_audio(&mut self) {
        self.engine.stop();
        self.engine = AudioEngine::start(&self.shared, &self.cfg, self.force_shared);
        self.audio_dirty = false;
        self.cfg.save();
    }

    fn connect(&mut self, target: String) {
        match self.shared.connect(&target) {
            Ok(addr) => {
                self.cfg.last_peer = target;
                self.cfg.save();
                self.message = Some((format!("Rufe {addr} …"), GREY));
            }
            Err(e) => self.message = Some((e, RED)),
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shared.disconnect();
        self.cfg.save();
    }
}

fn latency_color(ms: f32) -> Color32 {
    if ms <= 12.0 {
        GREEN
    } else if ms <= 25.0 {
        YELLOW
    } else {
        RED
    }
}

fn fmt_ms(ms: f32) -> String {
    if ms.is_nan() {
        "–".into()
    } else {
        format!("{ms:.1} ms")
    }
}

/// Pegelanzeige von -60 bis 0 dBFS, optional mit Schwellwert-Markierung.
fn meter(ui: &mut egui::Ui, level: f32, threshold_db: Option<f32>, active: bool) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 14.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let frac = |db: f32| ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let db = gain_to_db(level);
    let mut fill = rect;
    fill.set_width(rect.width() * frac(db));
    let color = if db > -3.0 {
        RED
    } else if active {
        GREEN
    } else {
        Color32::from_rgb(70, 130, 90)
    };
    p.rect_filled(fill, 3.0, color);
    if let Some(t) = threshold_db {
        let x = rect.left() + rect.width() * frac(t);
        p.line_segment([egui::pos2(x, rect.top() - 2.0), egui::pos2(x, rect.bottom() + 2.0)], Stroke::new(2.0_f32, YELLOW));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(40));
        let sh = self.shared.clone();

        // Pegel abholen (Spitzenwert seit dem letzten Bild) und langsam abfallen lassen.
        let pin = sh.in_peak.get();
        sh.in_peak.set(0.0);
        let pout = sh.out_peak.get();
        sh.out_peak.set(0.0);
        self.meter_in = pin.max(self.meter_in * 0.85);
        self.meter_out = pout.max(self.meter_out * 0.85);

        if let Some(n) = sh.notice.lock().unwrap().take() {
            self.message = Some((n, YELLOW));
        }
        if let Some(rate) = self.engine.measured_capture_rate(&sh) {
            if self.engine.is_exclusive() && AudioEngine::rate_is_broken(rate) {
                self.force_shared = true;
                self.restart_audio();
                self.message = Some((
                    format!("Der Treiber liefert im Exklusiv-Modus falsche Daten ({rate:.0} statt 48000 Samples/s). Nutze den geteilten Modus."),
                    YELLOW,
                ));
            }
        }
        if let Some(key) = sh.ptt.take_learned() {
            self.cfg.ptt_key = Some(key);
            self.cfg.save();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.connection_ui(ui, &sh);
                ui.add_space(6.0);
                self.talk_ui(ui, &sh);
                ui.add_space(6.0);
                self.listen_ui(ui, &sh);
                ui.add_space(6.0);
                self.latency_ui(ui, &sh);
                ui.add_space(6.0);
                self.settings_ui(ui);
            });
        });

        sh.apply_controls(&self.cfg);
    }
}

impl App {
    fn connection_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Verbindung");

            let (text, color) = match sh.peer() {
                None => ("Nicht verbunden".to_string(), GREY),
                Some(_) if sh.is_test() => ("Testmodus: du hörst dich selbst".to_string(), GREEN),
                Some(addr) => {
                    let name = sh.peer_name.lock().unwrap().clone();
                    let who = if name.is_empty() { addr.to_string() } else { format!("{name} ({addr})") };
                    if sh.peer_confirmed.load(Relaxed) {
                        (format!("Verbunden mit {who}"), GREEN)
                    } else if sh.seconds_since_heard() > 4.0 {
                        (format!("Keine Antwort von {who}. Läuft IPTalk dort? Firewall?"), RED)
                    } else {
                        (format!("Verbinde mit {who} …"), YELLOW)
                    }
                }
            };
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(RichText::new(text).strong());
            });

            ui.horizontal(|ui| {
                ui.label("IP des Kumpels:");
                let resp = ui.add(egui::TextEdit::singleline(&mut self.peer_input).hint_text("z. B. 192.168.1.20").desired_width(160.0));
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Verbinden").clicked() || enter {
                    self.connect(self.peer_input.clone());
                }
                if sh.peer().is_some() && ui.button("Trennen").clicked() {
                    sh.disconnect();
                    self.message = None;
                }
            });

            let found = sh.discovered.lock().unwrap().clone();
            if !found.is_empty() {
                ui.label("Im Netzwerk gefunden (anklicken zum Verbinden):");
                for d in found {
                    let label = format!("💻 {}  –  {}", d.name, d.addr);
                    if ui.button(label).clicked() {
                        self.peer_input = d.addr.ip().to_string();
                        self.connect(d.addr.to_string());
                    }
                }
            }

            ui.horizontal(|ui| {
                if ui.button("🎧 Testmodus (eigene Stimme hören)").clicked() {
                    sh.connect_test();
                    self.message = None;
                }
                ui.checkbox(&mut self.cfg.auto_accept, "Anrufe automatisch annehmen");
            });
            ui.horizontal(|ui| {
                ui.label("Dein Name:");
                ui.add(egui::TextEdit::singleline(&mut self.cfg.name).desired_width(140.0));
                ui.label(RichText::new(format!("Port {} (UDP)", sh.port)).color(GREY));
            });

            if let Some((m, c)) = &self.message {
                ui.colored_label(*c, m);
            }
        });
    }

    fn talk_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.heading("Sprechen");
                let sending = sh.sending.load(Relaxed);
                status_dot(ui, if sending { GREEN } else { GREY });
                ui.label(RichText::new(if sending { "sendet" } else { "still" }).color(if sending { GREEN } else { GREY }));
            });
            ui.horizontal(|ui| {
                for m in [TalkMode::Open, TalkMode::Gate, TalkMode::PushToTalk] {
                    ui.radio_value(&mut self.cfg.mode, m, m.label());
                }
            });

            let threshold = (self.cfg.mode == TalkMode::Gate).then_some(self.cfg.gate_threshold_db);
            meter(ui, self.meter_in, threshold, sh.sending.load(Relaxed));

            match self.cfg.mode {
                TalkMode::Gate => {
                    ui.add(egui::Slider::new(&mut self.cfg.gate_threshold_db, -70.0..=-10.0).suffix(" dB").text("Schwelle"));
                    ui.label(RichText::new("Gelbe Linie knapp über das Grundrauschen stellen (Lüfter, Tastatur).").color(GREY).small());
                }
                TalkMode::PushToTalk => {
                    ui.horizontal(|ui| {
                        let key = self.cfg.ptt_key.as_ref().map(|k| k.label.clone()).unwrap_or_else(|| "keine".into());
                        ui.label(format!("Taste: {key}"));
                        if sh.ptt.is_learning() {
                            ui.label(RichText::new("Jetzt Taste oder Maustaste drücken …").color(YELLOW));
                            if ui.button("Abbrechen").clicked() {
                                sh.ptt.cancel_learning();
                            }
                        } else if ui.button("Taste festlegen").clicked() {
                            sh.ptt.start_learning();
                        }
                        if sh.ptt.pressed.load(Relaxed) {
                            ui.label(RichText::new("gedrückt").color(GREEN));
                        }
                    });
                    if let Some(p) = sh.ptt.problem.lock().unwrap().as_ref() {
                        ui.colored_label(RED, p);
                    }
                }
                TalkMode::Open => {}
            }

            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.cfg.mic_gain_db, -20.0..=30.0).suffix(" dB").text("Mikro-Verstärkung"));
            });
            let mut muted = sh.mic_muted.load(Relaxed);
            let text = if muted { "🔇 Mikro stumm" } else { "🎤 Mikro an" };
            if ui.toggle_value(&mut muted, text).changed() {
                sh.mic_muted.store(muted, Relaxed);
            }
        });
    }

    fn listen_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Hören");
            meter(ui, self.meter_out, None, true);
            ui.add(egui::Slider::new(&mut self.cfg.out_gain_db, -30.0..=20.0).suffix(" dB").text("Lautstärke Kumpel"));
            let mut muted = sh.out_muted.load(Relaxed);
            let text = if muted { "🔇 Kumpel stumm" } else { "🔊 Kumpel hörbar" };
            if ui.toggle_value(&mut muted, text).changed() {
                sh.out_muted.store(muted, Relaxed);
            }
        });
    }

    fn latency_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Latenz");
            let mine = sh.my_to_you_ms.get();
            let theirs = sh.you_to_me_ms.get();
            egui::Grid::new("lat").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
                let (a, b) = if sh.is_test() {
                    ("Deine Stimme bei dir (Schleife)", "")
                } else {
                    ("Deine Stimme beim Kumpel", "Kumpels Stimme bei dir")
                };
                ui.label(a);
                ui.label(RichText::new(fmt_ms(mine)).strong().size(18.0).color(if mine.is_nan() { GREY } else { latency_color(mine) }));
                ui.end_row();
                if !b.is_empty() {
                    ui.label(b);
                    ui.label(RichText::new(fmt_ms(theirs)).strong().size(18.0).color(if theirs.is_nan() { GREY } else { latency_color(theirs) }));
                    ui.end_row();
                }
                ui.label("Ping (Hin und zurück)");
                ui.label(fmt_ms(sh.rtt_ms.get()));
                ui.end_row();
                ui.label("Jitterpuffer");
                let js = &sh.jitter.stats;
                ui.label(format!("{} (Reserve {})", fmt_ms(js.fill_ms.get()), fmt_ms(js.margin_ms.get())));
                ui.end_row();
                ui.label("Aussetzer / Verluste");
                ui.label(format!(
                    "{} Unterläufe, {} verloren, {} zu spät",
                    js.underruns.load(Relaxed),
                    js.lost.load(Relaxed),
                    js.late.load(Relaxed)
                ));
                ui.end_row();
            });
            ui.label(
                RichText::new("Gemessen von Mikro-Callback bis Kopfhörer-Callback, plus geschätzte Gerätepuffer. Werte erscheinen, sobald gesprochen wird.")
                    .color(GREY)
                    .small(),
            );
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Audio-Einstellungen").default_open(false).show(ui, |ui| {
            let before = (
                self.cfg.backend,
                self.cfg.capture_device.clone(),
                self.cfg.playback_device.clone(),
                self.cfg.profile,
                self.cfg.period_frames,
                self.cfg.exclusive_capture,
                self.cfg.exclusive_playback,
            );

            if Backend::available().len() > 1 {
                egui::ComboBox::from_label("Audio-System")
                    .selected_text(self.cfg.backend.label())
                    .show_ui(ui, |ui| {
                        for b in Backend::available() {
                            ui.selectable_value(&mut self.cfg.backend, *b, b.label());
                        }
                    });
            }
            device_combo(ui, "Mikrofon", &mut self.cfg.capture_device, &self.engine.capture_devices);
            device_combo(ui, "Kopfhörer", &mut self.cfg.playback_device, &self.engine.playback_devices);

            ui.label("Latenz-Profil:");
            for p in [Profile::Ultra, Profile::Stable, Profile::Manual] {
                ui.radio_value(&mut self.cfg.profile, p, p.label());
            }
            if self.cfg.profile == Profile::Manual {
                egui::ComboBox::from_label("Puffergröße")
                    .selected_text(format!("{} Frames ({:.1} ms)", self.cfg.period_frames, self.cfg.period_frames as f32 / 48.0))
                    .show_ui(ui, |ui| {
                        for f in [32u32, 64, 96, 128, 192, 256, 512] {
                            ui.selectable_value(&mut self.cfg.period_frames, f, format!("{f} Frames ({:.1} ms)", f as f32 / 48.0));
                        }
                    });
                if cfg!(windows) {
                    ui.checkbox(&mut self.cfg.exclusive_capture, "Mikrofon exklusiv (WASAPI Exclusive)");
                    ui.checkbox(&mut self.cfg.exclusive_playback, "Kopfhörer exklusiv – Achtung: Spielsound ist dann auf diesem Gerät stumm!");
                }
            }

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.cfg.jitter_auto, "Jitterpuffer automatisch");
                if !self.cfg.jitter_auto {
                    ui.add(egui::Slider::new(&mut self.cfg.jitter_ms, 0.0..=30.0).suffix(" ms").text("Reserve"));
                }
            });

            let after = (
                self.cfg.backend,
                self.cfg.capture_device.clone(),
                self.cfg.playback_device.clone(),
                self.cfg.profile,
                self.cfg.period_frames,
                self.cfg.exclusive_capture,
                self.cfg.exclusive_playback,
            );
            if before != after {
                self.audio_dirty = true;
            }
            if self.audio_dirty {
                if ui.button(RichText::new("Übernehmen (Audio neu starten)").strong()).clicked() {
                    self.force_shared = false;
                    self.restart_audio();
                }
            } else if ui.button("Audio neu starten / Geräte neu einlesen").clicked() {
                self.restart_audio();
            }

            ui.separator();
            ui.label(RichText::new(format!("Audio-System: {}", self.engine.backend_name)).color(GREY));
            if let Some(i) = self.engine.capture_info {
                ui.label(
                    RichText::new(format!(
                        "Mikrofon: {} Frames × {} @ {} Hz = {:.1} ms{}",
                        i.period,
                        i.periods,
                        i.rate,
                        i.period_ms(),
                        if i.exclusive { ", exklusiv" } else { "" }
                    ))
                    .color(GREY),
                );
            }
            if let Some(i) = self.engine.playback_info {
                ui.label(
                    RichText::new(format!(
                        "Kopfhörer: {} Frames × {} @ {} Hz = {:.1} ms{}",
                        i.period,
                        i.periods,
                        i.rate,
                        i.period_ms(),
                        if i.exclusive { ", exklusiv" } else { "" }
                    ))
                    .color(GREY),
                );
            }
            if cfg!(target_os = "linux") {
                ui.label(RichText::new("Hinweis: Das PipeWire-Quantum wird beim Programmstart gesetzt. Nach einem Profilwechsel IPTalk neu starten.").color(GREY).small());
            }
        });
        for e in &self.engine.errors {
            ui.colored_label(YELLOW, e);
        }
    }
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.5, color);
}

fn device_combo(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, devices: &[String]) {
    let shown = value.clone().unwrap_or_else(|| "Standardgerät".into());
    egui::ComboBox::from_label(label).selected_text(shown).width(300.0).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "Standardgerät");
        for d in devices {
            ui.selectable_value(value, Some(d.clone()), d);
        }
    });
}
