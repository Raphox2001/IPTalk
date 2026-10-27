//! Signalverarbeitung im Capture-Callback: Verstärkung, Pegel, Rauschsperre und PTT.

use crate::audio::SAMPLE_RATE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TalkMode {
    /// Mikrofon immer offen.
    Open,
    /// Immer offen, aber nur senden, wenn der Pegel über der Schwelle liegt.
    Gate,
    /// Nur senden, solange die PTT-Taste gedrückt ist.
    PushToTalk,
}

impl TalkMode {
    pub fn label(self) -> &'static str {
        match self {
            TalkMode::Open => "Immer offen",
            TalkMode::Gate => "Rauschsperre",
            TalkMode::PushToTalk => "Push-to-Talk",
        }
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 1e-6 {
        -120.0
    } else {
        20.0 * g.log10()
    }
}

/// Wie lange die Rauschsperre nach dem letzten lauten Block offen bleibt.
const GATE_HOLD: usize = SAMPLE_RATE as usize * 300 / 1000;
/// Nachlauf nach dem Loslassen der PTT-Taste, damit Wortenden nicht abgeschnitten werden.
const PTT_TAIL: usize = SAMPLE_RATE as usize * 120 / 1000;
const ATTACK_STEP: f32 = 1.0 / (SAMPLE_RATE as f32 * 0.001); // 1 ms
const RELEASE_STEP: f32 = 1.0 / (SAMPLE_RATE as f32 * 0.015); // 15 ms

pub struct Params {
    pub mode: TalkMode,
    pub gain: f32,
    pub gate_threshold_db: f32,
    pub ptt_pressed: bool,
    pub muted: bool,
}

pub struct Decision {
    /// Diesen Block senden?
    pub send: bool,
    /// Erster Block nach einer Pause.
    pub talkspurt: bool,
    /// Spitzenpegel (linear) nach der Verstärkung.
    pub peak: f32,
    /// Pegel über der Rauschsperren-Schwelle?
    pub above_threshold: bool,
}

#[derive(Default)]
pub struct Processor {
    env: f32,
    hold_left: usize,
    sending: bool,
}

impl Processor {
    /// Verarbeitet einen Block in-place und entscheidet, ob gesendet wird.
    pub fn process(&mut self, block: &mut [f32], p: &Params) -> Decision {
        let mut peak = 0f32;
        let mut sum = 0f32;
        for s in block.iter_mut() {
            *s *= p.gain;
            peak = peak.max(s.abs());
            sum += *s * *s;
        }
        let rms = (sum / block.len().max(1) as f32).sqrt();
        let above = gain_to_db(rms) >= p.gate_threshold_db;

        let open = if p.muted {
            self.hold_left = 0;
            false
        } else {
            match p.mode {
                TalkMode::Open => true,
                TalkMode::Gate => {
                    if above {
                        self.hold_left = GATE_HOLD;
                    } else {
                        self.hold_left = self.hold_left.saturating_sub(block.len());
                    }
                    self.hold_left > 0
                }
                TalkMode::PushToTalk => {
                    if p.ptt_pressed {
                        self.hold_left = PTT_TAIL;
                    } else {
                        self.hold_left = self.hold_left.saturating_sub(block.len());
                    }
                    self.hold_left > 0
                }
            }
        };

        // Weiches Ein-/Ausblenden, damit es beim Schalten nicht klickt.
        let target = if open { 1.0 } else { 0.0 };
        if !open && self.env == 0.0 {
            // Das Ende einer Sprechphase merkt der Empfänger am Ausbleiben der Pakete.
            self.sending = false;
            return Decision { send: false, talkspurt: false, peak, above_threshold: above };
        }
        for s in block.iter_mut() {
            if self.env < target {
                self.env = (self.env + ATTACK_STEP).min(1.0);
            } else if self.env > target {
                self.env = (self.env - RELEASE_STEP).max(0.0);
            }
            *s *= self.env;
        }
        let talkspurt = !self.sending;
        self.sending = true;
        Decision { send: true, talkspurt, peak, above_threshold: above }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(mode: TalkMode) -> Params {
        Params { mode, gain: 1.0, gate_threshold_db: -40.0, ptt_pressed: false, muted: false }
    }

    #[test]
    fn open_mode_always_sends() {
        let mut pr = Processor::default();
        let mut b = [0f32; 120];
        let d = pr.process(&mut b, &params(TalkMode::Open));
        assert!(d.send && d.talkspurt);
        let d = pr.process(&mut b, &params(TalkMode::Open));
        assert!(d.send && !d.talkspurt);
    }

    #[test]
    fn gate_opens_holds_and_closes() {
        let mut pr = Processor::default();
        let p = params(TalkMode::Gate);
        let mut quiet = [0.0001f32; 120];
        assert!(!pr.process(&mut quiet, &p).send);
        let mut loud = [0.5f32; 120];
        let d = pr.process(&mut loud, &p);
        assert!(d.send && d.talkspurt && d.above_threshold);
        // Hold: 300 ms bleibt die Sperre offen.
        let mut sent = 0;
        for _ in 0..400 {
            let mut q = [0.0001f32; 120];
            if pr.process(&mut q, &p).send {
                sent += 1;
            }
        }
        let hold_blocks = GATE_HOLD / 120;
        assert!(sent >= hold_blocks && sent <= hold_blocks + 10, "sent={sent}");
    }

    #[test]
    fn ptt_and_mute() {
        let mut pr = Processor::default();
        let mut p = params(TalkMode::PushToTalk);
        let mut b = [0.3f32; 120];
        assert!(!pr.process(&mut b, &p).send);
        p.ptt_pressed = true;
        assert!(pr.process(&mut b, &p).send);
        p.muted = true;
        // Stumm: ausblenden, dann nichts mehr senden.
        let mut sends = 0;
        for _ in 0..20 {
            let mut b = [0.3f32; 120];
            if pr.process(&mut b, &p).send {
                sends += 1;
            }
        }
        assert!(sends <= 7);
    }
}
