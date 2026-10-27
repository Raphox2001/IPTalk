//! Jitterpuffer mit Drift-Ausgleich.
//!
//! Ein Schreiber (Empfangsthread) und ein Leser (Playback-Callback) teilen sich einen
//! lock-freien Ringpuffer. Der Leser regelt den *minimalen* Füllstand (Reserve nach dem
//! Lesen) auf einen kleinen Zielwert: Netzwerk-Jitter wird so automatisch mit abgedeckt,
//! und weil zwei Soundkarten nie exakt gleich schnell laufen, wird bei Bedarf pro
//! Callback ein Sample weggelassen oder ergänzt (lineare Interpolation über den Block).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};

use crate::audio::SAMPLE_RATE;

const CAPACITY: usize = 1 << 15; // ~680 ms
const MASK: u64 = CAPACITY as u64 - 1;

/// Untergrenze der automatischen Reserve.
const AUTO_MARGIN_MIN_MS: f32 = 0.5;
const AUTO_MARGIN_MAX_MS: f32 = 30.0;
/// Regelfenster: alle 250 ms wird nachgeregelt.
const WINDOW: usize = SAMPLE_RATE as usize / 4;
/// Abweichung, die ohne Korrektur toleriert wird.
const HYSTERESIS: i64 = 16;
/// Höchstens so viele Korrekturen pro Fenster (≈ 0,4 % Tempo).
const MAX_ADJUST: i64 = 48;
/// Zu viel im Puffer → harter Sprung zurück auf den Zielwert.
const MAX_EXCESS: i64 = SAMPLE_RATE as i64 / 10;
/// Fehlende Pakete bis zu dieser Zahl werden mit Stille aufgefüllt.
const MAX_CONCEAL_PACKETS: u32 = 8;
const FADE: usize = 48;

fn ms_to_samples(ms: f32) -> i64 {
    (ms * SAMPLE_RATE as f32 / 1000.0).round() as i64
}

fn samples_to_ms(s: f32) -> f32 {
    s * 1000.0 / SAMPLE_RATE as f32
}

#[derive(Default)]
pub struct F32Cell(AtomicU32);

impl F32Cell {
    pub fn new(v: f32) -> F32Cell {
        F32Cell(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed)
    }
}

#[derive(Default)]
pub struct JitterStats {
    pub underruns: AtomicU64,
    pub lost: AtomicU64,
    pub late: AtomicU64,
    pub overflow: AtomicU64,
    pub corrections: AtomicU64,
    /// Durchschnittlicher Füllstand im letzten Fenster (ms) = Verzögerung durch den Puffer.
    pub fill_ms: F32Cell,
    /// Aktuelle Ziel-Reserve (ms).
    pub margin_ms: F32Cell,
    pub playing: AtomicBool,
}

pub struct JitterBuffer {
    buf: Box<[AtomicU32]>,
    write: AtomicU64,
    read: AtomicU64,
    /// Zeitstempel (Uhr des Absenders, µs) des zuletzt geschriebenen Samples.
    newest_ts: AtomicU64,
    /// Der Absender hat nach einer Pause wieder angefangen zu senden.
    talkspurt: AtomicBool,
    /// Feste Reserve in ms; negativ = automatisch.
    manual_margin_ms: F32Cell,
    pub stats: JitterStats,
}

/// Zustand des Schreibers (lebt im Empfangsthread).
#[derive(Default)]
pub struct WriterState {
    expected_seq: Option<u32>,
}

/// Zustand des Lesers (lebt im Playback-Callback).
pub struct ReaderState {
    playing: bool,
    pending_underrun: bool,
    auto_margin: f32,
    ok_windows: u32,
    window_min: i64,
    window_fill_sum: f64,
    window_len: usize,
    adjust: i64,
    /// Integralanteil des Reglers: gelernte Taktabweichung (Korrekturen pro Fenster).
    /// Bleibt über Sprechpausen hinweg erhalten.
    drift: f64,
    fade_in: usize,
}

impl Default for ReaderState {
    fn default() -> Self {
        ReaderState {
            playing: false,
            pending_underrun: false,
            auto_margin: 1.0,
            ok_windows: 0,
            window_min: i64::MAX,
            window_fill_sum: 0.0,
            window_len: 0,
            adjust: 0,
            drift: 0.0,
            fade_in: 0,
        }
    }
}

impl ReaderState {
    fn reset_window(&mut self) {
        self.window_min = i64::MAX;
        self.window_fill_sum = 0.0;
        self.window_len = 0;
    }
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl JitterBuffer {
    pub fn new() -> JitterBuffer {
        JitterBuffer {
            buf: (0..CAPACITY).map(|_| AtomicU32::new(0)).collect(),
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            newest_ts: AtomicU64::new(0),
            talkspurt: AtomicBool::new(false),
            manual_margin_ms: F32Cell::new(-1.0),
            stats: JitterStats::default(),
        }
    }

    /// Aktueller Füllstand in Samples (nur zur Anzeige/Diagnose).
    pub fn fill_samples(&self) -> u64 {
        self.write.load(Relaxed).saturating_sub(self.read.load(Relaxed))
    }

    /// Bisher insgesamt geschriebene/gelesene Samples (Diagnose).
    pub fn totals(&self) -> (u64, u64) {
        (self.write.load(Relaxed), self.read.load(Relaxed))
    }

    /// `None` = automatisch, sonst feste Reserve in ms.
    pub fn set_manual_margin(&self, ms: Option<f32>) {
        self.manual_margin_ms.set(ms.unwrap_or(-1.0));
    }

    fn store(&self, pos: u64, v: f32) {
        self.buf[(pos & MASK) as usize].store(v.to_bits(), Relaxed);
    }

    fn load(&self, pos: u64) -> f32 {
        f32::from_bits(self.buf[(pos & MASK) as usize].load(Relaxed))
    }

    /// Vom Empfangsthread aufgerufen. `ts_last` ist der Aufnahmezeitpunkt des letzten Samples.
    pub fn push(&self, w: &mut WriterState, seq: u32, talkspurt: bool, ts_last: u64, samples: &[f32]) {
        let mut conceal = 0usize;
        match w.expected_seq {
            Some(exp) if !talkspurt => {
                let diff = seq.wrapping_sub(exp) as i32;
                if diff < 0 {
                    self.stats.late.fetch_add(1, Relaxed);
                    return;
                }
                if diff > 0 {
                    self.stats.lost.fetch_add(diff as u64, Relaxed);
                    if (diff as u32) <= MAX_CONCEAL_PACKETS {
                        conceal = diff as usize * samples.len();
                    }
                }
            }
            _ => {}
        }
        w.expected_seq = Some(seq.wrapping_add(1));
        if talkspurt {
            self.talkspurt.store(true, Relaxed);
        }

        let wr = self.write.load(Relaxed);
        let rd = self.read.load(Acquire);
        let used = (wr - rd) as usize;
        if used + conceal + samples.len() > CAPACITY {
            self.stats.overflow.fetch_add(1, Relaxed);
            return;
        }
        let mut pos = wr;
        for _ in 0..conceal {
            self.store(pos, 0.0);
            pos += 1;
        }
        for &s in samples {
            self.store(pos, s);
            pos += 1;
        }
        self.write.store(pos, Release);
        self.newest_ts.store(ts_last, Release);
    }

    fn margin_samples(&self, r: &ReaderState) -> i64 {
        let manual = self.manual_margin_ms.get();
        let ms = if manual >= 0.0 { manual } else { r.auto_margin };
        ms_to_samples(ms).max(1)
    }

    /// Vom Playback-Callback aufgerufen. Gibt den Absender-Zeitstempel des ersten
    /// abgespielten Samples zurück (für die Latenzmessung), falls gerade etwas spielt.
    pub fn pull(&self, r: &mut ReaderState, out: &mut [f32]) -> Option<u64> {
        let n = out.len();
        if n == 0 {
            return None;
        }
        let ts_newest = self.newest_ts.load(Acquire);
        let wr = self.write.load(Acquire);
        let mut rd = self.read.load(Relaxed);
        let fill = (wr - rd) as i64;
        let margin = self.margin_samples(r);

        if !r.playing {
            if fill < margin + n as i64 {
                out.fill(0.0);
                self.stats.playing.store(false, Relaxed);
                return None;
            }
            // Wieder Audio da. War das vorherige Ende ein echter Unterlauf (kein neuer Sprechbeginn)?
            // Die Sprechbeginn-Markierung erst hier abholen, sonst geht sie beim Vorpuffern verloren.
            let spurt = self.talkspurt.swap(false, Relaxed);
            if r.pending_underrun && !spurt {
                self.stats.underruns.fetch_add(1, Relaxed);
                r.auto_margin = (r.auto_margin + 0.5).min(AUTO_MARGIN_MAX_MS);
                r.ok_windows = 0;
            }
            r.pending_underrun = false;
            r.playing = true;
            r.fade_in = FADE;
            r.adjust = r.drift.round() as i64;
            r.reset_window();
            // Nur so viel vorhalten wie nötig: überzählige Samples überspringen.
            let excess = fill - margin - n as i64;
            if excess > 0 {
                rd += excess as u64;
            }
        }
        let fill = (wr - rd) as i64;
        let played_ts = ts_newest.saturating_sub((fill as u64) * 1_000_000 / SAMPLE_RATE as u64);

        if fill < n as i64 {
            // Unterlauf: Rest mit Ausblenden abspielen, dann Stille.
            let avail = fill.max(0) as usize;
            for (i, o) in out.iter_mut().enumerate().take(avail) {
                let g = (avail - i) as f32 / avail as f32;
                *o = self.load(rd + i as u64) * g;
            }
            out[avail..].fill(0.0);
            self.read.store(rd + avail as u64, Release);
            r.playing = false;
            r.pending_underrun = true;
            self.stats.playing.store(false, Relaxed);
            return Some(played_ts);
        }

        // Läuft durchgehend: eine Sprechbeginn-Markierung ohne Pause dazwischen ist bedeutungslos.
        self.talkspurt.store(false, Relaxed);

        // Regelung: Reserve nach diesem Block messen.
        let headroom = fill - n as i64;
        r.window_min = r.window_min.min(headroom);
        r.window_fill_sum += fill as f64 * n as f64;
        r.window_len += n;

        if headroom - margin > MAX_EXCESS {
            rd = wr - (margin + n as i64) as u64;
            r.reset_window();
        }

        // Ein Sample weglassen (adjust > 0) oder ergänzen (adjust < 0).
        let src_len = if r.adjust > 0 && headroom > 1 {
            r.adjust -= 1;
            self.stats.corrections.fetch_add(1, Relaxed);
            n + 1
        } else if r.adjust < 0 && n > 2 {
            r.adjust += 1;
            self.stats.corrections.fetch_add(1, Relaxed);
            n - 1
        } else {
            n
        };

        if src_len == n {
            for (i, o) in out.iter_mut().enumerate() {
                *o = self.load(rd + i as u64);
            }
        } else {
            let step = (src_len - 1) as f64 / (n - 1).max(1) as f64;
            for (i, o) in out.iter_mut().enumerate() {
                let p = i as f64 * step;
                let k = p.floor() as u64;
                let frac = (p - k as f64) as f32;
                let a = self.load(rd + k);
                let b = if (k + 1) < src_len as u64 { self.load(rd + k + 1) } else { a };
                *o = a + (b - a) * frac;
            }
        }
        if r.fade_in > 0 {
            for o in out.iter_mut() {
                if r.fade_in == 0 {
                    break;
                }
                *o *= 1.0 - r.fade_in as f32 / FADE as f32;
                r.fade_in -= 1;
            }
        }
        self.read.store(rd + src_len as u64, Release);
        self.stats.playing.store(true, Relaxed);

        if r.window_len >= WINDOW {
            let e = r.window_min - margin;
            // PI-Regler: P reagiert auf Abweichungen, I lernt die konstante Drift der Soundkarten.
            let p = if e.abs() <= HYSTERESIS { 0.0 } else { e as f64 / 4.0 };
            r.drift = (r.drift + e as f64 / 16.0).clamp(-(MAX_ADJUST as f64), MAX_ADJUST as f64);
            r.adjust = ((p + r.drift).round() as i64).clamp(-MAX_ADJUST, MAX_ADJUST);
            self.stats
                .fill_ms
                .set(samples_to_ms((r.window_fill_sum / r.window_len as f64) as f32));
            // Reserve langsam wieder abbauen, wenn es lange keine Unterläufe gab (Auto-Modus).
            r.ok_windows += 1;
            if r.ok_windows >= 80 {
                r.ok_windows = 0;
                r.auto_margin = (r.auto_margin - 0.25).max(AUTO_MARGIN_MIN_MS);
            }
            self.stats.margin_ms.set(samples_to_ms(self.margin_samples(r) as f32));
            r.reset_window();
        }
        Some(played_ts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Einfacher deterministischer Zufallsgenerator für die Simulation.
    struct Lcg(u64);
    impl Lcg {
        fn next_f64(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Simuliert Sender und Empfänger mit verschiedenen Takten und Netzwerk-Jitter.
    /// Gibt (Unterläufe nach Einschwingen, max. Füllstand in ms nach Einschwingen) zurück.
    fn simulate(ppm: f64, jitter_ms: f64, seconds: f64, packet: usize, period: usize) -> (u64, f32) {
        let jb = JitterBuffer::new();
        let mut w = WriterState::default();
        let mut r = ReaderState::default();
        let mut rng = Lcg(1234);

        let sender_rate = SAMPLE_RATE as f64 * (1.0 + ppm * 1e-6);
        let packet_dt = packet as f64 / sender_rate;
        let period_dt = period as f64 / SAMPLE_RATE as f64;

        // Ankunftszeiten der Pakete (mit Jitter, Reihenfolge bleibt erhalten wie im LAN).
        let mut next_send = 0.0;
        let mut seq = 0u32;
        let mut next_pull = 0.002;
        let mut out = vec![0f32; period];
        let samples: Vec<f32> = (0..packet).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
        let mut pending: std::collections::VecDeque<(f64, u32)> = Default::default();
        let mut last_arrival = 0.0f64;
        let mut max_fill_ms = 0.0f32;
        let mut underruns_at_settle = None;
        let settle = 5.0;

        while next_pull < seconds {
            while next_send <= next_pull + 0.05 {
                let arrival = (next_send + 0.0003 + rng.next_f64() * jitter_ms / 1000.0).max(last_arrival);
                last_arrival = arrival;
                pending.push_back((arrival, seq));
                seq = seq.wrapping_add(1);
                next_send += packet_dt;
            }
            while let Some(&(t, s)) = pending.front() {
                if t > next_pull {
                    break;
                }
                pending.pop_front();
                jb.push(&mut w, s, s == 0, (t * 1e6) as u64, &samples);
            }
            jb.pull(&mut r, &mut out);
            if next_pull > settle {
                if underruns_at_settle.is_none() {
                    underruns_at_settle = Some(jb.stats.underruns.load(Relaxed));
                }
                let fill = (jb.write.load(Relaxed) - jb.read.load(Relaxed)) as f32;
                max_fill_ms = max_fill_ms.max(samples_to_ms(fill));
            }
            next_pull += period_dt;
        }
        let under = jb.stats.underruns.load(Relaxed) - underruns_at_settle.unwrap_or(0);
        eprintln!(
            "ppm={ppm} jitter={jitter_ms}ms: Unterläufe={under} max. Füllstand={max_fill_ms:.2}ms Ø={:.2}ms Reserve={:.2}ms Korrekturen={}",
            jb.stats.fill_ms.get(),
            jb.stats.margin_ms.get(),
            jb.stats.corrections.load(Relaxed)
        );
        (under, max_fill_ms)
    }

    #[test]
    fn drift_fast_sender_stays_bounded() {
        // Sender 500 ppm schneller: ohne Ausgleich wären das nach 10 min 300 ms mehr Latenz.
        let (under, max_fill) = simulate(500.0, 0.3, 600.0, 120, 64);
        assert_eq!(under, 0);
        assert!(max_fill < 8.0, "Füllstand {max_fill} ms");
    }

    #[test]
    fn drift_slow_sender_stays_bounded() {
        let (under, max_fill) = simulate(-500.0, 0.3, 600.0, 120, 64);
        assert_eq!(under, 0);
        assert!(max_fill < 8.0, "Füllstand {max_fill} ms");
    }

    #[test]
    fn large_jitter_adapts() {
        // 3 ms Jitter: die automatische Reserve wächst, danach keine Unterläufe mehr.
        let (_, max_fill) = simulate(100.0, 3.0, 120.0, 120, 128);
        assert!(max_fill < 15.0, "Füllstand {max_fill} ms");
    }

    #[test]
    fn lost_and_late_packets() {
        let jb = JitterBuffer::new();
        let mut w = WriterState::default();
        let p = [0.25f32; 120];
        jb.push(&mut w, 10, true, 0, &p);
        jb.push(&mut w, 12, false, 0, &p); // 11 fehlt → 120 Samples Stille
        jb.push(&mut w, 11, false, 0, &p); // zu spät
        jb.push(&mut w, 12, false, 0, &p); // doppelt
        assert_eq!(jb.stats.lost.load(Relaxed), 1);
        assert_eq!(jb.stats.late.load(Relaxed), 2);
        assert_eq!(jb.write.load(Relaxed), 360);
    }

    #[test]
    fn talkspurt_after_pause_is_not_an_underrun() {
        let jb = JitterBuffer::new();
        let mut w = WriterState::default();
        let mut r = ReaderState::default();
        let p = [0.1f32; 64];
        let mut out = [0f32; 64];
        let mut seq = 0;
        for _ in 0..3 {
            // Sprechphase: ein paar Pakete, alles abspielen.
            for i in 0..10 {
                jb.push(&mut w, seq, i == 0, 0, &p);
                seq += 1;
                jb.pull(&mut r, &mut out);
            }
            // Pause: Puffer läuft leer (Unterlauf, der kein Fehler ist).
            for _ in 0..5 {
                jb.pull(&mut r, &mut out);
            }
            // Neuer Sprechbeginn kommt an, während noch vorgepuffert wird.
            jb.push(&mut w, seq, true, 0, &p[..16]);
            seq += 1;
            jb.pull(&mut r, &mut out);
        }
        assert_eq!(jb.stats.underruns.load(Relaxed), 0);
    }

    #[test]
    fn manual_margin_is_used() {
        let jb = JitterBuffer::new();
        jb.set_manual_margin(Some(5.0));
        let r = ReaderState::default();
        assert_eq!(jb.margin_samples(&r), 240);
    }

    #[test]
    fn plays_back_samples_in_order() {
        let jb = JitterBuffer::new();
        let mut w = WriterState::default();
        let mut r = ReaderState::default();
        jb.set_manual_margin(Some(0.0));
        let data: Vec<f32> = (0..120).map(|i| i as f32 / 1000.0).collect();
        jb.push(&mut w, 0, true, 0, &data);
        let mut out = [0f32; 64];
        jb.pull(&mut r, &mut out);
        // Reserve 1 Sample → 120 - 1 - 64 = 55 überzählige Samples werden übersprungen.
        // Nach dem Einblenden (48 Samples) kommen die Samples unverändert der Reihe nach.
        for i in 48..64 {
            assert_eq!(out[i], data[55 + i]);
        }
    }
}
