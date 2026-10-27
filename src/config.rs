//! Einstellungen, gespeichert als TOML im Konfigurationsordner des Benutzers.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio::Backend;
use crate::dsp::TalkMode;

pub const DEFAULT_PORT: u16 = 47800;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// Kleinste Puffer im geteilten Low-Latency-Modus.
    Ultra,
    /// Größere Puffer, falls es knackst.
    Stable,
    /// Werte aus den Einzel-Einstellungen.
    Manual,
}

impl Profile {
    pub fn label(self) -> &'static str {
        match self {
            Profile::Ultra => "Ultra (kleinste Latenz)",
            Profile::Stable => "Stabil (falls es knackst)",
            Profile::Manual => "Manuell",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PttKey {
    /// Windows: Virtual-Key-Code, Linux: evdev-Tastencode.
    pub code: u32,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub name: String,
    pub port: u16,
    pub last_peer: String,
    pub auto_accept: bool,

    pub backend: Backend,
    pub capture_device: Option<String>,
    pub playback_device: Option<String>,
    pub profile: Profile,
    pub period_frames: u32,
    pub exclusive_capture: bool,
    pub exclusive_playback: bool,
    pub jitter_auto: bool,
    pub jitter_ms: f32,

    pub mode: TalkMode,
    pub gate_threshold_db: f32,
    pub mic_gain_db: f32,
    pub out_gain_db: f32,
    pub ptt_key: Option<PttKey>,

    /// Beim Start nach einer neuen Version suchen.
    pub check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: default_name(),
            port: DEFAULT_PORT,
            last_peer: String::new(),
            auto_accept: true,
            backend: Backend::Auto,
            capture_device: None,
            playback_device: None,
            profile: Profile::Ultra,
            period_frames: 128,
            exclusive_capture: false,
            exclusive_playback: false,
            jitter_auto: true,
            jitter_ms: 2.0,
            mode: TalkMode::Gate,
            gate_threshold_db: -45.0,
            mic_gain_db: 0.0,
            out_gain_db: 0.0,
            ptt_key: None,
            check_updates: true,
        }
    }
}

/// Tatsächlich zu verwendende Audio-Parameter (Profil aufgelöst).
pub struct AudioParams {
    pub period: u32,
    pub exclusive_capture: bool,
    pub exclusive_playback: bool,
}

impl Config {
    pub fn audio_params(&self) -> AudioParams {
        match self.profile {
            // Geteilter Low-Latency-Modus (WASAPI IAudioClient3 bzw. kleines PipeWire-Quantum).
            // Der Kopfhörer bleibt immer geteilt, sonst wären andere Anwendungen (z. B. Spiele) auf dem Gerät nicht mehr hörbar.
            Profile::Ultra => AudioParams { period: 64, exclusive_capture: false, exclusive_playback: false },
            Profile::Stable => AudioParams { period: 256, exclusive_capture: false, exclusive_playback: false },
            Profile::Manual => AudioParams {
                period: self.period_frames,
                exclusive_capture: cfg!(windows) && self.exclusive_capture,
                exclusive_playback: cfg!(windows) && self.exclusive_playback,
            },
        }
    }

    fn path() -> Option<PathBuf> {
        let dir = dirs::config_dir()?;
        Some(if cfg!(windows) { dir.join("IPTalk") } else { dir.join("iptalk") }.join("config.toml"))
    }

    pub fn load() -> Config {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }
}

fn default_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    host.unwrap_or_else(|| "IPTalk".to_string())
}
