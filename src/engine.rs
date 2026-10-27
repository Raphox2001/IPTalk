//! Gemeinsamer Zustand, Netzwerk-Threads und Audio-Geräte.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::*};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio::{Backend, Context, Stream, StreamInfo, SAMPLE_RATE};
use crate::config::Config;
use crate::dsp::{self, Processor, TalkMode};
use crate::jitter::{F32Cell, JitterBuffer, ReaderState, WriterState};
use crate::protocol::{self as proto, Kind, Stats, MAX_PACKET, MAX_SAMPLES};
use crate::ptt::Ptt;

#[derive(Clone, Debug)]
pub struct Discovered {
    pub addr: SocketAddr,
    pub name: String,
    last_seen: Instant,
}

pub struct Shared {
    pub socket: UdpSocket,
    pub port: u16,
    start: Instant,
    instance_id: u32,
    pub name: Mutex<String>,

    /// Gegenstelle, kodiert als 1<<48 | IPv4<<16 | Port; 0 = keine.
    peer: AtomicU64,
    pub peer_name: Mutex<String>,
    pub peer_confirmed: AtomicBool,
    last_heard_ms: AtomicU64,
    pub auto_accept: AtomicBool,
    pub discovered: Mutex<Vec<Discovered>>,
    pub notice: Mutex<Option<String>>,

    pub jitter: JitterBuffer,
    pub ptt: Arc<Ptt>,

    // Bedienung
    mode: AtomicU8,
    pub mic_gain: F32Cell,
    pub out_gain: F32Cell,
    pub gate_db: F32Cell,
    pub mic_muted: AtomicBool,
    pub out_muted: AtomicBool,

    // Anzeigen
    pub in_peak: F32Cell,
    pub out_peak: F32Cell,
    pub sending: AtomicBool,
    pub above_threshold: AtomicBool,
    pub tx_packets: AtomicU64,
    pub tx_samples: AtomicU64,
    /// Vom Mikrofon gelieferte Samples (für die Samplerate-Prüfung).
    pub cap_frames: AtomicU64,
    pub play_samples: AtomicU64,
    pub rx_packets: AtomicU64,

    // Latenz (ms; NaN = unbekannt)
    pub rtt_ms: F32Cell,
    pub my_to_you_ms: F32Cell,
    pub you_to_me_ms: F32Cell,
    pub capture_dev_ms: F32Cell,
    pub playback_dev_ms: F32Cell,
    peer_playback_dev_ms: F32Cell,
}

fn encode_peer(a: SocketAddrV4) -> u64 {
    1 << 48 | (u32::from(*a.ip()) as u64) << 16 | a.port() as u64
}

fn decode_peer(v: u64) -> Option<SocketAddr> {
    if v == 0 {
        return None;
    }
    let ip = Ipv4Addr::from(((v >> 16) & 0xFFFF_FFFF) as u32);
    Some(SocketAddr::V4(SocketAddrV4::new(ip, (v & 0xFFFF) as u16)))
}

fn ema(cell: &F32Cell, v: f32, k: f32) {
    let old = cell.get();
    cell.set(if old.is_nan() { v } else { old + (v - old) * k });
}

impl Shared {
    pub fn new(cfg: &Config, port: u16, ptt: Arc<Ptt>) -> std::io::Result<Arc<Shared>> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let start = Instant::now();
        let instance_id = (std::process::id() ^ start.elapsed().as_nanos() as u32 ^ 0x9E37_79B9).max(1);

        let sh = Arc::new(Shared {
            socket,
            port,
            start,
            instance_id,
            name: Mutex::new(cfg.name.clone()),
            peer: AtomicU64::new(0),
            peer_name: Mutex::new(String::new()),
            peer_confirmed: AtomicBool::new(false),
            last_heard_ms: AtomicU64::new(0),
            auto_accept: AtomicBool::new(cfg.auto_accept),
            discovered: Mutex::new(Vec::new()),
            notice: Mutex::new(None),
            jitter: JitterBuffer::new(),
            ptt,
            mode: AtomicU8::new(0),
            mic_gain: F32Cell::new(1.0),
            out_gain: F32Cell::new(1.0),
            gate_db: F32Cell::new(-45.0),
            mic_muted: AtomicBool::new(false),
            out_muted: AtomicBool::new(false),
            in_peak: F32Cell::new(0.0),
            out_peak: F32Cell::new(0.0),
            sending: AtomicBool::new(false),
            above_threshold: AtomicBool::new(false),
            tx_packets: AtomicU64::new(0),
            tx_samples: AtomicU64::new(0),
            cap_frames: AtomicU64::new(0),
            play_samples: AtomicU64::new(0),
            rx_packets: AtomicU64::new(0),
            rtt_ms: F32Cell::new(f32::NAN),
            my_to_you_ms: F32Cell::new(f32::NAN),
            you_to_me_ms: F32Cell::new(f32::NAN),
            capture_dev_ms: F32Cell::new(0.0),
            playback_dev_ms: F32Cell::new(0.0),
            peer_playback_dev_ms: F32Cell::new(f32::NAN),
        });
        sh.apply_controls(cfg);

        let s = sh.clone();
        std::thread::Builder::new().name("net-recv".into()).spawn(move || recv_loop(s))?;
        let s = sh.clone();
        std::thread::Builder::new().name("net-tick".into()).spawn(move || tick_loop(s))?;
        Ok(sh)
    }

    /// Übernimmt Regler-Werte aus der Konfiguration.
    pub fn apply_controls(&self, cfg: &Config) {
        self.mode.store(
            match cfg.mode {
                TalkMode::Open => 0,
                TalkMode::Gate => 1,
                TalkMode::PushToTalk => 2,
            },
            Relaxed,
        );
        self.mic_gain.set(dsp::db_to_gain(cfg.mic_gain_db));
        self.out_gain.set(dsp::db_to_gain(cfg.out_gain_db));
        self.gate_db.set(cfg.gate_threshold_db);
        self.auto_accept.store(cfg.auto_accept, Relaxed);
        self.jitter.set_manual_margin(if cfg.jitter_auto { None } else { Some(cfg.jitter_ms) });
        *self.name.lock().unwrap() = cfg.name.clone();
    }

    fn mode(&self) -> TalkMode {
        match self.mode.load(Relaxed) {
            0 => TalkMode::Open,
            1 => TalkMode::Gate,
            _ => TalkMode::PushToTalk,
        }
    }

    pub fn now_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn peer(&self) -> Option<SocketAddr> {
        decode_peer(self.peer.load(Acquire))
    }

    fn set_peer(&self, addr: Option<SocketAddrV4>) {
        self.peer.store(addr.map(encode_peer).unwrap_or(0), Release);
        self.peer_confirmed.store(false, Relaxed);
        self.last_heard_ms.store(self.now_ms(), Relaxed);
        self.rtt_ms.set(f32::NAN);
        self.my_to_you_ms.set(f32::NAN);
        self.you_to_me_ms.set(f32::NAN);
        self.peer_playback_dev_ms.set(f32::NAN);
        self.peer_name.lock().unwrap().clear();
    }

    /// Verbindet mit `target` ("192.168.1.20" oder "192.168.1.20:47800" oder Hostname).
    pub fn connect(&self, target: &str) -> Result<SocketAddr, String> {
        let target = target.trim();
        if target.is_empty() {
            return Err("Bitte die IP-Adresse deines Kumpels eingeben.".into());
        }
        let with_port = if target.contains(':') { target.to_string() } else { format!("{target}:{}", self.port) };
        let addr = with_port
            .to_socket_addrs()
            .map_err(|e| format!("Adresse „{target}“ ungültig: {e}"))?
            .find_map(|a| match a {
                SocketAddr::V4(v4) => Some(v4),
                _ => None,
            })
            .ok_or_else(|| format!("Für „{target}“ gibt es keine IPv4-Adresse."))?;
        if let Some(old) = self.peer() {
            if old != SocketAddr::V4(addr) {
                self.send_simple(Kind::Bye, old);
            }
        }
        self.set_peer(Some(addr));
        self.send_hello(SocketAddr::V4(addr));
        Ok(SocketAddr::V4(addr))
    }

    /// Eigene Stimme über den kompletten Netzwerkweg zurückhören.
    pub fn connect_test(&self) {
        let _ = self.connect(&format!("127.0.0.1:{}", self.port));
    }

    pub fn disconnect(&self) {
        if let Some(p) = self.peer() {
            self.send_simple(Kind::Bye, p);
        }
        self.set_peer(None);
    }

    pub fn is_test(&self) -> bool {
        matches!(self.peer(), Some(SocketAddr::V4(a)) if a.ip().is_loopback() && a.port() == self.port)
    }

    fn send_simple(&self, kind: Kind, to: SocketAddr) {
        let mut buf = [0u8; proto::HEADER_LEN];
        proto::write_header(&mut buf, kind, 0, self.instance_id, self.now_us());
        let _ = self.socket.send_to(&buf, to);
    }

    fn send_hello(&self, to: SocketAddr) {
        let name = self.name.lock().unwrap().clone();
        let _ = self.socket.send_to(&proto::encode_named(Kind::Hello, self.instance_id, self.now_us(), &name), to);
    }

    pub fn seconds_since_heard(&self) -> f32 {
        (self.now_ms().saturating_sub(self.last_heard_ms.load(Relaxed))) as f32 / 1000.0
    }
}

fn recv_loop(sh: Arc<Shared>) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }

    let mut buf = [0u8; 2048];
    let mut samples = [0f32; 1024];
    let mut writer = WriterState::default();
    let mut writer_peer = 0u64;

    loop {
        let (n, from) = match sh.socket.recv_from(&mut buf) {
            Ok(x) => x,
            // Timeout, oder unter Windows „Connection reset“ nach ICMP-Fehlern: einfach weiter.
            Err(_) => continue,
        };
        let Some((h, payload)) = proto::parse(&buf[..n]) else { continue };
        let SocketAddr::V4(from4) = from else { continue };

        let peer_raw = sh.peer.load(Acquire);
        if peer_raw != writer_peer {
            writer = WriterState::default();
            writer_peer = peer_raw;
        }
        let is_peer = peer_raw == encode_peer(from4);
        let free = peer_raw == 0 && sh.auto_accept.load(Relaxed);

        match h.kind {
            Kind::Audio => {
                if !is_peer {
                    if !free {
                        continue;
                    }
                    sh.set_peer(Some(from4));
                    writer = WriterState::default();
                    writer_peer = sh.peer.load(Acquire);
                    sh.send_hello(from);
                }
                sh.last_heard_ms.store(sh.now_ms(), Relaxed);
                sh.rx_packets.fetch_add(1, Relaxed);
                let k = proto::decode_audio(payload, &mut samples);
                sh.jitter.push(&mut writer, h.seq, h.flags & proto::FLAG_TALKSPURT != 0, h.ts, &samples[..k]);
            }
            Kind::Hello => {
                if !is_peer && !free {
                    continue; // schon mit jemand anderem verbunden
                }
                if !is_peer {
                    sh.set_peer(Some(from4));
                }
                sh.peer_confirmed.store(true, Relaxed);
                sh.last_heard_ms.store(sh.now_ms(), Relaxed);
                *sh.peer_name.lock().unwrap() = proto::decode_name(payload);
                let name = sh.name.lock().unwrap().clone();
                let _ = sh.socket.send_to(&proto::encode_named(Kind::HelloAck, sh.instance_id, sh.now_us(), &name), from);
            }
            Kind::HelloAck if is_peer => {
                sh.peer_confirmed.store(true, Relaxed);
                sh.last_heard_ms.store(sh.now_ms(), Relaxed);
                *sh.peer_name.lock().unwrap() = proto::decode_name(payload);
            }
            Kind::Discover if h.seq != sh.instance_id => {
                let name = sh.name.lock().unwrap().clone();
                let _ = sh.socket.send_to(&proto::encode_named(Kind::Announce, sh.instance_id, sh.now_us(), &name), from);
            }
            Kind::Announce if h.seq != sh.instance_id => {
                let name = proto::decode_name(payload);
                let mut list = sh.discovered.lock().unwrap();
                match list.iter_mut().find(|d| d.addr == from) {
                    Some(d) => {
                        d.name = name;
                        d.last_seen = Instant::now();
                    }
                    None => list.push(Discovered { addr: from, name, last_seen: Instant::now() }),
                }
            }
            Kind::Ping if is_peer => {
                sh.last_heard_ms.store(sh.now_ms(), Relaxed);
                let mut out = [0u8; proto::HEADER_LEN];
                proto::write_header(&mut out, Kind::Pong, 0, h.seq, h.ts);
                let _ = sh.socket.send_to(&out, from);
            }
            Kind::Pong if is_peer => {
                sh.last_heard_ms.store(sh.now_ms(), Relaxed);
                let rtt = sh.now_us().saturating_sub(h.ts) as f32 / 1000.0;
                ema(&sh.rtt_ms, rtt, 0.2);
            }
            Kind::Played if is_peer => {
                // h.ts = Aufnahmezeit (unsere Uhr) des Samples, das der Kumpel gerade abspielt.
                let loop_ms = sh.now_us().saturating_sub(h.ts) as f32 / 1000.0;
                let rtt = sh.rtt_ms.get();
                let back = if rtt.is_nan() { 0.0 } else { rtt / 2.0 };
                let peer_dev = sh.peer_playback_dev_ms.get();
                let peer_dev = if peer_dev.is_nan() { 0.0 } else { peer_dev };
                let total = sh.capture_dev_ms.get() + (loop_ms - back).max(0.0) + peer_dev;
                if total < 2000.0 {
                    ema(&sh.my_to_you_ms, total, 0.1);
                }
            }
            Kind::Stats if is_peer => {
                if let Some(s) = proto::decode_stats(payload) {
                    sh.peer_playback_dev_ms.set(s.playback_device_ms);
                    sh.you_to_me_ms.set(s.my_to_you_ms);
                }
            }
            Kind::Bye if is_peer => {
                sh.set_peer(None);
                *sh.notice.lock().unwrap() = Some("Dein Kumpel hat die Verbindung getrennt.".into());
            }
            _ => {}
        }
    }
}

fn tick_loop(sh: Arc<Shared>) {
    let broadcast = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, sh.port));
    let mut tick: u64 = 0;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        tick += 1;

        if let Some(peer) = sh.peer() {
            if !sh.peer_confirmed.load(Relaxed) || tick % 8 == 0 {
                sh.send_hello(peer);
            }
            sh.send_simple(Kind::Ping, peer);
            if tick % 2 == 0 {
                let stats = Stats { playback_device_ms: sh.playback_dev_ms.get(), my_to_you_ms: sh.my_to_you_ms.get() };
                let _ = sh.socket.send_to(&proto::encode_stats(sh.instance_id, sh.now_us(), &stats), peer);
            }
            if sh.seconds_since_heard() > 4.0 {
                sh.peer_confirmed.store(false, Relaxed);
            }
        }

        // LAN-Suche alle 2 s.
        if tick % 8 == 1 {
            sh.send_simple(Kind::Discover, broadcast);
            sh.discovered.lock().unwrap().retain(|d| d.last_seen.elapsed() < Duration::from_secs(7));
        }
    }
}

/// Die geöffneten Audio-Geräte. Feld-Reihenfolge wichtig: Streams vor dem Kontext freigeben.
pub struct AudioEngine {
    capture: Option<Stream>,
    playback: Option<Stream>,
    ctx: Option<Context>,
    pub capture_info: Option<StreamInfo>,
    pub playback_info: Option<StreamInfo>,
    pub backend_name: String,
    pub errors: Vec<String>,
    pub capture_devices: Vec<String>,
    pub playback_devices: Vec<String>,
    started: Instant,
    cap_frames_at_start: u64,
    rate_checked: bool,
}

/// Erlaubte Abweichung der gemessenen Mikrofon-Samplerate.
const RATE_TOLERANCE: f32 = 0.03;

impl AudioEngine {
    /// `force_shared`: Exklusiv-Modus ignorieren (nach einer fehlgeschlagenen Samplerate-Prüfung).
    pub fn start(sh: &Arc<Shared>, cfg: &Config, force_shared: bool) -> AudioEngine {
        let mut eng = AudioEngine {
            capture: None,
            playback: None,
            ctx: None,
            capture_info: None,
            playback_info: None,
            backend_name: String::new(),
            errors: Vec::new(),
            capture_devices: Vec::new(),
            playback_devices: Vec::new(),
            started: Instant::now(),
            cap_frames_at_start: sh.cap_frames.load(Relaxed),
            rate_checked: false,
        };
        let backend = if Backend::available().contains(&cfg.backend) { cfg.backend } else { Backend::Auto };
        let ctx = match Context::new(backend) {
            Ok(c) => c,
            Err(e) => {
                eng.errors.push(format!("Audio-System „{}“ nicht verfügbar: {e}", backend.label()));
                return eng;
            }
        };
        eng.backend_name = ctx.backend_name();
        eng.capture_devices = ctx.devices(true).into_iter().map(|d| d.name).collect();
        eng.playback_devices = ctx.devices(false).into_iter().map(|d| d.name).collect();
        let mut p = cfg.audio_params();
        if force_shared {
            p.exclusive_capture = false;
            p.exclusive_playback = false;
        }

        // Mikrofon
        let open_capture = |exclusive: bool| {
            let sh = sh.clone();
            let mut proc = Processor::default();
            let mut seq: u32 = 0;
            let mut packet = [0u8; MAX_PACKET];
            let mut block = [0f32; MAX_SAMPLES];
            ctx.open_capture(cfg.capture_device.as_deref(), p.period, exclusive, move |input| {
                let now = sh.now_us();
                sh.cap_frames.fetch_add(input.len() as u64, Relaxed);
                let params = dsp::Params {
                    mode: sh.mode(),
                    gain: sh.mic_gain.get(),
                    gate_threshold_db: sh.gate_db.get(),
                    ptt_pressed: sh.ptt.pressed.load(Relaxed),
                    muted: sh.mic_muted.load(Relaxed),
                };
                let peer = sh.peer();
                let total = input.len();
                let mut done = 0;
                let mut peak = 0f32;
                let mut sending = false;
                let mut above = false;
                for chunk in input.chunks(MAX_SAMPLES) {
                    let b = &mut block[..chunk.len()];
                    b.copy_from_slice(chunk);
                    let d = proc.process(b, &params);
                    done += chunk.len();
                    peak = peak.max(d.peak);
                    above |= d.above_threshold;
                    if d.send {
                        sending = true;
                        if let Some(peer) = peer {
                            let ts_last = now.saturating_sub(((total - done) as u64) * 1_000_000 / SAMPLE_RATE as u64);
                            let flags = if d.talkspurt { proto::FLAG_TALKSPURT } else { 0 };
                            let n = proto::encode_audio(&mut packet, flags, seq, ts_last, b);
                            if sh.socket.send_to(&packet[..n], peer).is_ok() {
                                sh.tx_packets.fetch_add(1, Relaxed);
                                sh.tx_samples.fetch_add(b.len() as u64, Relaxed);
                            }
                            seq = seq.wrapping_add(1);
                        }
                    }
                }
                sh.in_peak.set(sh.in_peak.get().max(peak));
                sh.sending.store(sending && peer.is_some(), Relaxed);
                sh.above_threshold.store(above, Relaxed);
            })
        };
        let cap = match open_capture(p.exclusive_capture) {
            Err(e) if p.exclusive_capture => {
                eng.errors.push(format!("Mikrofon exklusiv nicht möglich ({e}), nutze geteilten Modus."));
                open_capture(false)
            }
            r => r,
        };
        match cap {
            Ok(s) => {
                let info = s.info();
                sh.capture_dev_ms.set(info.device_latency_ms(true));
                eng.capture_info = Some(info);
                eng.capture = Some(s);
            }
            Err(e) => eng.errors.push(format!("Mikrofon konnte nicht geöffnet werden: {e}")),
        }

        // Kopfhörer
        let open_playback = |exclusive: bool| {
            let sh = sh.clone();
            let mut reader = ReaderState::default();
            let mut since_report = 0usize;
            let mut packet = [0u8; proto::HEADER_LEN];
            ctx.open_playback(cfg.playback_device.as_deref(), p.period, exclusive, move |out| {
                let played = sh.jitter.pull(&mut reader, out);
                sh.play_samples.fetch_add(out.len() as u64, Relaxed);
                let g = if sh.out_muted.load(Relaxed) { 0.0 } else { sh.out_gain.get() };
                let mut peak = 0f32;
                for s in out.iter_mut() {
                    *s = (*s * g).clamp(-1.0, 1.0);
                    peak = peak.max(s.abs());
                }
                sh.out_peak.set(sh.out_peak.get().max(peak));

                // Etwa 10x pro Sekunde zurückmelden, was gerade gespielt wird (Latenzmessung).
                since_report += out.len();
                if let Some(ts) = played {
                    if since_report >= SAMPLE_RATE as usize / 10 {
                        since_report = 0;
                        if let Some(peer) = sh.peer() {
                            proto::write_header(&mut packet, Kind::Played, 0, 0, ts);
                            let _ = sh.socket.send_to(&packet, peer);
                        }
                    }
                }
            })
        };
        let play = match open_playback(p.exclusive_playback) {
            Err(e) if p.exclusive_playback => {
                eng.errors.push(format!("Kopfhörer exklusiv nicht möglich ({e}), nutze geteilten Modus."));
                open_playback(false)
            }
            r => r,
        };
        match play {
            Ok(s) => {
                let info = s.info();
                sh.playback_dev_ms.set(info.device_latency_ms(false));
                eng.playback_info = Some(info);
                eng.playback = Some(s);
            }
            Err(e) => eng.errors.push(format!("Kopfhörer konnte nicht geöffnet werden: {e}")),
        }

        for (s, what) in [(&eng.capture, "Mikrofon"), (&eng.playback, "Kopfhörer")] {
            if let Some(s) = s {
                if let Err(e) = s.start() {
                    eng.errors.push(format!("{what} konnte nicht gestartet werden: {e}"));
                }
            }
        }
        eng.ctx = Some(ctx);
        eng.started = Instant::now();
        eng.cap_frames_at_start = sh.cap_frames.load(Relaxed);
        eng
    }

    pub fn is_exclusive(&self) -> bool {
        self.capture_info.map(|i| i.exclusive).unwrap_or(false) || self.playback_info.map(|i| i.exclusive).unwrap_or(false)
    }

    /// Misst einmalig (nach 2 s) die tatsächliche Mikrofon-Samplerate.
    /// Manche Treiber liefern im Exklusiv-Modus falsch getaktete Daten.
    pub fn measured_capture_rate(&mut self, sh: &Shared) -> Option<f32> {
        if self.rate_checked || self.capture.is_none() {
            return None;
        }
        let secs = self.started.elapsed().as_secs_f32();
        if secs < 2.0 {
            return None;
        }
        self.rate_checked = true;
        Some((sh.cap_frames.load(Relaxed) - self.cap_frames_at_start) as f32 / secs)
    }

    /// `true`, wenn die gemessene Rate so weit daneben liegt, dass der Exklusiv-Modus defekt ist.
    pub fn rate_is_broken(rate: f32) -> bool {
        ((rate - SAMPLE_RATE as f32) / SAMPLE_RATE as f32).abs() > RATE_TOLERANCE
    }

    /// Schließt alle Geräte (vor einem Neustart mit anderen Einstellungen).
    pub fn stop(&mut self) {
        self.capture = None;
        self.playback = None;
        self.ctx = None;
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
