// Unter Windows kein zusätzliches Konsolenfenster im Release-Build.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod config;
mod dsp;
mod engine;
mod jitter;
mod protocol;
mod ptt;
mod ui;
mod update;

use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;

const HELP: &str = "\
IPTalk – Sprachverbindung mit minimaler Latenz

Aufruf: iptalk [Optionen]

  --port <N>         anderen UDP-Port benutzen (Standard 47800)
  --selftest         ohne Fenster: Audio öffnen, verbinden und Messwerte ausgeben
  --connect <IP>     beim Selbsttest mit dieser Gegenstelle verbinden
                     (ohne Angabe: eigene Stimme über 127.0.0.1)
  --seconds <N>      Dauer des Selbsttests (Standard 10)
  --period <N>       Puffergröße in Frames erzwingen (z. B. 64, 128, 256)
  --exclusive        Mikrofon exklusiv öffnen (WASAPI Exclusive, Windows)
  --mic-off          beim Selbsttest nichts senden (nur empfangen und messen)
  --list-devices     Audio-Geräte auflisten
  --update           nach einer neuen Version suchen und sie installieren
  --version          Versionsnummer anzeigen
  --help             diese Hilfe
";

struct Args {
    port: Option<u16>,
    selftest: bool,
    connect: Option<String>,
    seconds: u64,
    period: Option<u32>,
    exclusive: bool,
    mic_off: bool,
    list_devices: bool,
    update: bool,
    version: bool,
    help: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { port: None, selftest: false, connect: None, seconds: 10, period: None, exclusive: false, mic_off: false, list_devices: false, update: false, version: false, help: false };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} braucht einen Wert"));
        match arg.as_str() {
            "--port" => a.port = Some(value("--port")?.parse().map_err(|_| "ungültiger Port")?),
            "--selftest" => a.selftest = true,
            "--connect" => a.connect = Some(value("--connect")?),
            "--seconds" => a.seconds = value("--seconds")?.parse().map_err(|_| "ungültige Dauer")?,
            "--period" => a.period = Some(value("--period")?.parse().map_err(|_| "ungültige Puffergröße")?),
            "--exclusive" => a.exclusive = true,
            "--mic-off" => a.mic_off = true,
            "--list-devices" => a.list_devices = true,
            "--update" => a.update = true,
            "--version" | "-V" => a.version = true,
            "--help" | "-h" => a.help = true,
            other => return Err(format!("Unbekannte Option: {other}")),
        }
    }
    Ok(a)
}

/// Im Release-Build unter Windows hat das Programm keine Konsole; für die
/// Kommandozeilen-Modi an die Konsole des Aufrufers anhängen.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

fn main() -> eframe::Result<()> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            attach_console();
            eprintln!("{e}\n\n{HELP}");
            std::process::exit(2);
        }
    };
    update::cleanup_previous();
    let cfg = config::Config::load();

    // PipeWire (auch über die JACK-API) liest das gewünschte Quantum aus der Umgebung.
    // Muss vor dem Öffnen der Audio-Geräte passieren.
    #[cfg(target_os = "linux")]
    if std::env::var_os("PIPEWIRE_LATENCY").is_none() {
        std::env::set_var("PIPEWIRE_LATENCY", format!("{}/{}", cfg.audio_params().period, audio::SAMPLE_RATE));
    }

    if args.help || args.list_devices || args.selftest || args.update || args.version {
        attach_console();
        if args.help {
            println!("{HELP}");
        } else if args.version {
            println!("IPTalk {}", update::CURRENT_VERSION);
        } else if args.update {
            std::process::exit(cli_update());
        } else if args.list_devices {
            list_devices();
        } else {
            std::process::exit(selftest(cfg, &args));
        }
        return Ok(());
    }

    let port = args.port.unwrap_or(cfg.port);
    let ptt = ptt::Ptt::start(cfg.ptt_key.as_ref());
    let shared = engine::Shared::new(&cfg, port, ptt);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("IPTalk")
            .with_inner_size([500.0, 780.0])
            .with_min_inner_size([420.0, 400.0]),
        ..Default::default()
    };

    match shared {
        Ok(shared) => {
            let result = eframe::run_native(
                "IPTalk",
                options,
                Box::new(move |_cc| Ok(Box::new(ui::App::new(cfg, shared)))),
            );
            // Nach einem Update: neue Version starten. Sie wartet, bis dieser Prozess den Port freigibt.
            if update::RESTART_REQUESTED.load(Relaxed) {
                update::relaunch();
            }
            result
        }
        Err(e) => {
            let msg = format!(
                "Port {port} (UDP) konnte nicht geöffnet werden: {e}\n\n\
                 Läuft IPTalk schon? Sonst mit --port einen anderen Port wählen."
            );
            eframe::run_simple_native("IPTalk – Fehler", options, move |ctx, _| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("IPTalk kann nicht starten");
                    ui.label(&msg);
                });
            })
        }
    }
}

fn cli_update() -> i32 {
    println!("Installierte Version: {}", update::current_version());
    match update::check() {
        Ok(None) => {
            println!("IPTalk ist auf dem neuesten Stand.");
            0
        }
        Ok(Some(rel)) => {
            println!("Neue Version {} gefunden, wird heruntergeladen …", rel.version);
            let mut last = 0;
            let r = update::install(&rel, |done, total| {
                let pct = if total > 0 { done * 100 / total } else { 0 };
                if pct >= last + 10 {
                    last = pct;
                    println!("  {pct} %");
                }
            });
            match r {
                Ok(()) => {
                    println!("Version {} installiert. Beim nächsten Start ist sie aktiv.", rel.version);
                    0
                }
                Err(e) => {
                    eprintln!("Update fehlgeschlagen: {e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("Suche nach Updates fehlgeschlagen: {e}");
            1
        }
    }
}

fn list_devices() {
    for b in audio::Backend::available() {
        match audio::Context::new(*b) {
            Ok(ctx) => {
                println!("== {} ({})", b.label(), ctx.backend_name());
                for (capture, what) in [(true, "Mikrofone"), (false, "Kopfhörer/Lautsprecher")] {
                    println!("  {what}:");
                    for d in ctx.devices(capture) {
                        println!("    - {}", d.name);
                    }
                }
            }
            Err(e) => println!("== {}: nicht verfügbar ({e})", b.label()),
        }
    }
}

fn selftest(mut cfg: config::Config, args: &Args) -> i32 {
    cfg.mode = dsp::TalkMode::Open;
    if args.period.is_some() || args.exclusive {
        let p = cfg.audio_params();
        cfg.profile = config::Profile::Manual;
        cfg.period_frames = args.period.unwrap_or(p.period);
        cfg.exclusive_capture = args.exclusive;
        cfg.exclusive_playback = false;
    }
    let port = args.port.unwrap_or(cfg.port);
    let ptt = ptt::Ptt::start(None);
    let sh = match engine::Shared::new(&cfg, port, ptt) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Port {port} (UDP) kann nicht geöffnet werden: {e}");
            return 1;
        }
    };
    // Ausgabe stumm, damit es über Lautsprecher keine Rückkopplung gibt. Gemessen wird trotzdem.
    sh.out_muted.store(true, Relaxed);
    sh.mic_muted.store(args.mic_off, Relaxed);
    let mut eng = engine::AudioEngine::start(&sh, &cfg, false);
    println!("Audio-System: {}", eng.backend_name);
    for (name, info) in [("Mikrofon ", eng.capture_info), ("Kopfhörer", eng.playback_info)] {
        match info {
            Some(i) => println!(
                "{name}: {} Frames x {} @ {} Hz = {:.2} ms{}",
                i.period,
                i.periods,
                i.rate,
                i.period_ms(),
                if i.exclusive { " (exklusiv)" } else { "" }
            ),
            None => println!("{name}: nicht geöffnet"),
        }
    }
    for e in &eng.errors {
        println!("Hinweis: {e}");
    }

    match &args.connect {
        Some(t) => match sh.connect(t) {
            Ok(a) => println!("Verbindungsaufbau zu {a} …"),
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
        None => {
            sh.connect_test();
            println!("Testmodus: eigene Stimme wird über 127.0.0.1:{port} zurückgespielt");
        }
    }

    // Exklusiv-Modus: nach 2 s die echte Mikrofon-Samplerate prüfen.
    if eng.is_exclusive() {
        report(&sh, 2);
        if let Some(rate) = eng.measured_capture_rate(&sh) {
            println!("Mikrofon-Samplerate gemessen: {rate:.0} Hz");
            if engine::AudioEngine::rate_is_broken(rate) {
                println!("Exklusiv-Modus liefert falsch getaktete Daten → wechsle in den geteilten Modus.");
                drop(eng);
                eng = engine::AudioEngine::start(&sh, &cfg, true);
            }
        }
    }
    report(&sh, args.seconds);
    let js = &sh.jitter.stats;
    // Empfangene Audiopakete bedeuten: Verbindung stand und der Weg funktioniert.
    let ok = eng.capture_info.is_some() && eng.playback_info.is_some() && sh.rx_packets.load(Relaxed) > 0;
    println!(
        "\nErgebnis: {}  (gesendet {}, empfangen {}, Unterläufe {}, verloren {})",
        if ok { "OK" } else { "FEHLER" },
        sh.tx_packets.load(Relaxed),
        sh.rx_packets.load(Relaxed),
        js.underruns.load(Relaxed),
        js.lost.load(Relaxed)
    );
    sh.disconnect();
    drop(eng);
    if ok {
        0
    } else {
        1
    }
}

fn report(sh: &Arc<engine::Shared>, seconds: u64) {
    let fmt = |v: f32| if v.is_nan() { "  –  ".to_string() } else { format!("{v:5.1}") };
    for s in 1..=seconds {
        std::thread::sleep(Duration::from_secs(1));
        let js = &sh.jitter.stats;
        println!(
            "[{s:>3}s] verbunden={} {:<16} senden={} ms  empfangen={} ms  ping={} ms  puffer={} ms  pegel={:6.1} dB  unterläufe={}",
            if sh.peer_confirmed.load(Relaxed) { "ja  " } else { "nein" },
            sh.peer_name.lock().unwrap().chars().take(16).collect::<String>(),
            fmt(sh.my_to_you_ms.get()),
            fmt(sh.you_to_me_ms.get()),
            fmt(sh.rtt_ms.get()),
            fmt(js.fill_ms.get()),
            dsp::gain_to_db(sh.in_peak.get()),
            js.underruns.load(Relaxed),
        );
        if std::env::var_os("IPTALK_DEBUG").is_some() {
            let (w, r) = sh.jitter.totals();
            println!(
                "        debug: cap_frames={} tx_samples={} play_samples={} jb_write={w} jb_read={r} fill={} korrekturen={}",
                sh.cap_frames.load(Relaxed),
                sh.tx_samples.load(Relaxed),
                sh.play_samples.load(Relaxed),
                sh.jitter.fill_samples(),
                js.corrections.load(Relaxed)
            );
        }
        sh.in_peak.set(0.0);
    }
}
