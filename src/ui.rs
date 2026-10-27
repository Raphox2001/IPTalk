//! Das Fenster.

use std::sync::atomic::Ordering::*;
use std::sync::Arc;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

use crate::audio::Backend;
use crate::config::{Config, Profile};
use crate::dsp::{gain_to_db, TalkMode};
use crate::config::DEFAULT_PORT;
use crate::engine::{AudioEngine, Shared};
use crate::netinfo::{self, LocalAddr};
use crate::update::{self, Updater};

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
    updater: Updater,
    /// Die Update-Suche wurde von Hand gestartet → auch „aktuell“/Fehler deutlich anzeigen.
    manual_update_check: bool,
    /// Eigene IP-Adressen, die wahrscheinlichste zuerst (alle paar Sekunden aktualisiert).
    local_addrs: Vec<LocalAddr>,
    addrs_updated: Instant,
    /// Zuletzt in den Verlauf übernommene Verbindung (Adresse, Name), um nicht jedes Bild zu speichern.
    recorded_peer: Option<(String, String)>,
}

impl App {
    pub fn new(cfg: Config, shared: Arc<Shared>) -> App {
        let engine = AudioEngine::start(&shared, &cfg, false);
        let updater = Updater::new();
        if cfg.check_updates {
            updater.check_in_background();
        }
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
            updater,
            manual_update_check: false,
            local_addrs: netinfo::local_ipv4(),
            addrs_updated: Instant::now(),
            recorded_peer: None,
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
                self.message = Some((format!("Verbindungsaufbau zu {addr} …"), GREY));
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
                    format!("Der Treiber liefert im Exklusiv-Modus falsche Daten ({rate:.0} statt 48000 Samples/s). Es wird der geteilte Modus verwendet."),
                    YELLOW,
                ));
            }
        }
        if let Some(key) = sh.ptt.take_learned() {
            self.cfg.ptt_key = Some(key);
            self.cfg.save();
        }

        self.update_panels(ctx);

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
        self.record_recent_peer(&sh);
    }
}

impl App {
    /// Hinweisleiste oben (neue Version) und Fußzeile mit Versionsnummer.
    fn update_panels(&mut self, ctx: &egui::Context) {
        let state = self.updater.state();
        let banner = match &state {
            update::State::Available(_) | update::State::Downloading { .. } | update::State::Installed(_) => true,
            update::State::Failed(_) => self.manual_update_check,
            _ => false,
        };
        if banner {
            egui::TopBottomPanel::top("update").show(ctx, |ui| {
                ui.add_space(4.0);
                match &state {
                    update::State::Available(rel) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "Version {} ist verfügbar (installiert: {}).",
                                    rel.version,
                                    update::current_version()
                                ))
                                .strong(),
                            );
                            if ui.button("Jetzt aktualisieren").clicked() {
                                self.updater.install_in_background(rel.clone());
                            }
                            ui.hyperlink_to("Versionshinweise", &rel.page_url);
                        });
                    }
                    update::State::Downloading { version, done, total } => {
                        let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                        ui.label(format!("Version {version} wird heruntergeladen …"));
                        ui.add(egui::ProgressBar::new(frac).show_percentage());
                    }
                    update::State::Installed(version) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("Version {version} wurde installiert.")).strong().color(GREEN));
                            if ui.button("Jetzt neu starten").clicked() {
                                update::RESTART_REQUESTED.store(true, Relaxed);
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                        if self.shared.peer().is_some() {
                            ui.label(RichText::new("Die aktuelle Verbindung wird beim Neustart kurz unterbrochen.").color(GREY).small());
                        }
                    }
                    update::State::Failed(e) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(RED, format!("Aktualisierung fehlgeschlagen: {e}"));
                            ui.hyperlink_to("Download-Seite öffnen", update::RELEASES_URL);
                        });
                    }
                    _ => {}
                }
                ui.add_space(4.0);
            });
        }

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("IPTalk {}", update::CURRENT_VERSION)).color(GREY));
                let busy = self.updater.busy();
                if ui.add_enabled(!busy, egui::Button::new("Nach Updates suchen").small()).clicked() {
                    self.manual_update_check = true;
                    self.updater.check_in_background();
                }
                match &state {
                    update::State::Checking => {
                        ui.spinner();
                    }
                    update::State::UpToDate if self.manual_update_check => {
                        ui.label(RichText::new("Die installierte Version ist aktuell.").color(GREEN).small());
                    }
                    update::State::Failed(_) if !self.manual_update_check => {
                        ui.label(RichText::new("Update-Prüfung nicht möglich").color(GREY).small());
                    }
                    _ => {}
                }
                ui.checkbox(&mut self.cfg.check_updates, RichText::new("Beim Start prüfen").small());
                #[cfg(target_os = "linux")]
                if !crate::desktop::desktop_entry_exists() && ui.small_button("Ins Startmenü aufnehmen").clicked() {
                    self.message = Some(match crate::desktop::install_desktop_entry() {
                        Ok(_) => ("IPTalk wurde ins Startmenü aufgenommen.".into(), GREEN),
                        Err(e) => (format!("Startmenü-Eintrag fehlgeschlagen: {e}"), RED),
                    });
                }
            });
        });
    }

    /// Zeigt die eigene IP-Adresse, die die Gegenstelle zum Verbinden eingeben muss.
    fn own_address_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        if self.addrs_updated.elapsed() > Duration::from_secs(5) {
            self.local_addrs = netinfo::local_ipv4();
            self.addrs_updated = Instant::now();
        }
        let port_suffix = if sh.port == DEFAULT_PORT { String::new() } else { format!(":{}", sh.port) };
        let Some(main) = self.local_addrs.first().cloned() else {
            ui.colored_label(RED, "Keine Netzwerkverbindung gefunden.");
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Eigene IP-Adresse:");
            let text = format!("{}{port_suffix}", main.ip);
            ui.label(RichText::new(&text).strong().monospace().size(16.0))
                .on_hover_text(format!("Netzwerkadapter: {}", main.interface));
            if ui.small_button("Kopieren").clicked() {
                ui.ctx().copy_text(text.clone());
                self.message = Some((format!("{text} in die Zwischenablage kopiert."), GREY));
            }
        });
        if self.local_addrs.len() > 1 {
            let others: Vec<String> = self.local_addrs[1..]
                .iter()
                .map(|a| format!("{}{port_suffix} ({})", a.ip, a.interface))
                .collect();
            ui.label(RichText::new(format!("Weitere Adressen: {}", others.join(", "))).color(GREY).small());
        }
        ui.label(RichText::new("Diese Adresse auf der Gegenstelle eingeben, falls sie nicht automatisch gefunden wird.").color(GREY).small());
    }

    /// Bestätigte Verbindungen in „Zuletzt verbunden“ übernehmen.
    fn record_recent_peer(&mut self, sh: &Arc<Shared>) {
        let Some(addr) = sh.peer() else {
            self.recorded_peer = None;
            return;
        };
        // Lokale Adressen (Testmodus, zweite Instanz auf demselben Rechner) nicht merken.
        if addr.ip().is_loopback() || !sh.peer_confirmed.load(Relaxed) {
            return;
        }
        let key = (addr.to_string(), sh.peer_name.lock().unwrap().clone());
        if self.recorded_peer.as_ref() != Some(&key) {
            self.cfg.remember_peer(&key.1, &key.0, unix_now());
            self.cfg.save();
            self.recorded_peer = Some(key);
        }
    }

    /// Liste „Zuletzt verbunden“: ein Klick verbindet erneut.
    fn recent_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>, found: &[crate::engine::Discovered]) {
        if self.cfg.recent.is_empty() {
            return;
        }
        ui.add_space(2.0);
        ui.label("Zuletzt verbunden:");
        let current = sh.peer().map(|a| a.to_string());
        let now = unix_now();
        let mut connect_to = None;
        let mut forget = None;
        egui::Grid::new("recent").num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
            for r in &self.cfg.recent {
                // Online, wenn die Gegenstelle gerade per LAN-Suche gefunden wird (auch unter neuer IP).
                let online = found.iter().find(|d| matches_recent(r, d)).map(|d| d.addr.to_string());
                let connected = current.as_deref() == Some(r.address.as_str())
                    || (online.is_some() && current == online);
                status_dot(ui, if connected || online.is_some() { GREEN } else { GREY });
                let title = if r.name.is_empty() { short_addr(&r.address) } else { r.name.clone() };
                if connected {
                    ui.label(RichText::new(title).strong());
                } else if ui
                    .button(title)
                    .on_hover_text(if online.is_some() { "Im Netzwerk gefunden. Klicken zum Verbinden." } else { "Klicken zum Verbinden." })
                    .clicked()
                {
                    connect_to = Some(online.clone().unwrap_or_else(|| r.address.clone()));
                }
                let addr_now = online.as_deref().unwrap_or(&r.address);
                let when = if connected { "verbunden".to_string() } else { relative_time(now.saturating_sub(r.last_used)) };
                ui.label(RichText::new(format!("{}  ·  {when}", short_addr(addr_now))).color(GREY).small());
                if ui.small_button("×").on_hover_text("Aus der Liste entfernen").clicked() {
                    forget = Some(r.address.clone());
                }
                ui.end_row();
            }
        });
        if let Some(a) = forget {
            self.cfg.forget_peer(&a);
            self.cfg.save();
        }
        if let Some(target) = connect_to {
            self.peer_input = short_addr(&target);
            self.connect(target);
        }
        ui.add_space(2.0);
    }

    fn connection_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Verbindung");

            let (text, color) = match sh.peer() {
                None => ("Nicht verbunden".to_string(), GREY),
                Some(_) if sh.is_test() => ("Testmodus: eigene Stimme wird zurückgespielt".to_string(), GREEN),
                Some(addr) => {
                    let name = sh.peer_name.lock().unwrap().clone();
                    let who = if name.is_empty() { addr.to_string() } else { format!("{name} ({addr})") };
                    if sh.peer_confirmed.load(Relaxed) {
                        (format!("Verbunden mit {who}"), GREEN)
                    } else if sh.seconds_since_heard() > 4.0 {
                        (format!("Keine Antwort von {who}. Bitte prüfen, ob IPTalk dort läuft und die Firewall UDP-Port {} freigibt.", sh.port), RED)
                    } else {
                        (format!("Verbindungsaufbau zu {who} …"), YELLOW)
                    }
                }
            };
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(RichText::new(text).strong());
            });
            self.own_address_ui(ui, sh);

            ui.horizontal(|ui| {
                ui.label("IP-Adresse der Gegenstelle:");
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
            self.recent_ui(ui, sh, &found);

            // Gefundene Rechner, die nicht schon unter „Zuletzt verbunden“ stehen.
            let new_found: Vec<_> = found
                .iter()
                .filter(|d| !self.cfg.recent.iter().any(|r| matches_recent(r, d)))
                .collect();
            if !new_found.is_empty() {
                ui.label("Im lokalen Netzwerk gefunden (zum Verbinden anklicken):");
                for d in new_found {
                    let label = format!("💻 {}  –  {}", d.name, short_addr(&d.addr.to_string()));
                    if ui.button(label).clicked() {
                        self.peer_input = d.addr.ip().to_string();
                        self.connect(d.addr.to_string());
                    }
                }
            }

            if ui.button("🎧 Testmodus (eigene Stimme zurückspielen)").clicked() {
                sh.connect_test();
                self.message = None;
            }
            ui.checkbox(&mut self.cfg.auto_accept, "Eingehende Verbindungen automatisch annehmen");
            ui.horizontal(|ui| {
                ui.label("Anzeigename:");
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
                    ui.label(RichText::new("Schwelle (gelbe Markierung) knapp oberhalb des Grundrauschens einstellen.").color(GREY).small());
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
                ui.add(egui::Slider::new(&mut self.cfg.mic_gain_db, -20.0..=30.0).suffix(" dB").text("Mikrofon-Verstärkung"));
            });
            let mut muted = sh.mic_muted.load(Relaxed);
            let text = if muted { "🔇 Mikrofon stumm" } else { "🎤 Mikrofon aktiv" };
            if ui.toggle_value(&mut muted, text).changed() {
                sh.mic_muted.store(muted, Relaxed);
            }
        });
    }

    fn listen_ui(&mut self, ui: &mut egui::Ui, sh: &Arc<Shared>) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Wiedergabe");
            meter(ui, self.meter_out, None, true);
            ui.add(egui::Slider::new(&mut self.cfg.out_gain_db, -30.0..=20.0).suffix(" dB").text("Lautstärke Gegenstelle"));
            let mut muted = sh.out_muted.load(Relaxed);
            let text = if muted { "🔇 Gegenstelle stumm" } else { "🔊 Gegenstelle hörbar" };
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
                    ("Schleife (Senden + Empfangen)", "")
                } else {
                    ("Senderichtung", "Empfangsrichtung")
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
                RichText::new("Gemessen vom Aufnahme- bis zum Wiedergabe-Callback der Gegenseite, zuzüglich geschätzter Gerätepuffer. Werte erscheinen, sobald Audio übertragen wird.")
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
                    ui.checkbox(&mut self.cfg.exclusive_playback, "Kopfhörer exklusiv (andere Anwendungen sind auf diesem Gerät dann nicht mehr hörbar)");
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
                ui.label(RichText::new("Hinweis: Das PipeWire-Quantum wird beim Programmstart gesetzt. Nach einem Profilwechsel ist ein Neustart von IPTalk erforderlich.").color(GREY).small());
            }
        });
        for e in &self.engine.errors {
            ui.colored_label(YELLOW, e);
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// IP-Adresse ohne den Standard-Port („192.168.0.20:47800“ → „192.168.0.20“).
fn short_addr(addr: &str) -> String {
    match addr.parse::<SocketAddr>() {
        Ok(a) if a.port() == DEFAULT_PORT => a.ip().to_string(),
        _ => addr.to_string(),
    }
}

/// Gehört ein per LAN-Suche gefundener Rechner zu diesem Verlaufseintrag?
fn matches_recent(r: &crate::config::RecentPeer, d: &crate::engine::Discovered) -> bool {
    d.addr.to_string() == r.address || (!r.name.is_empty() && d.name.eq_ignore_ascii_case(&r.name))
}

fn relative_time(secs: u64) -> String {
    match secs {
        0..=59 => "gerade eben".into(),
        60..=3599 => format!("vor {} Min.", secs / 60),
        3600..=86_399 => format!("vor {} Std.", secs / 3600),
        86_400..=172_799 => "gestern".into(),
        _ => format!("vor {} Tagen", secs / 86_400),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(5), "gerade eben");
        assert_eq!(relative_time(125), "vor 2 Min.");
        assert_eq!(relative_time(7200), "vor 2 Std.");
        assert_eq!(relative_time(90_000), "gestern");
        assert_eq!(relative_time(3 * 86_400), "vor 3 Tagen");
    }

    #[test]
    fn short_addresses() {
        assert_eq!(short_addr("192.168.0.20:47800"), "192.168.0.20");
        assert_eq!(short_addr("192.168.0.20:47801"), "192.168.0.20:47801");
    }
}
