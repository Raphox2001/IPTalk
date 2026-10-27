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

/// Eine Gegenstelle, mit der schon einmal eine Verbindung bestand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentPeer {
    /// Anzeigename der Gegenstelle (kann leer sein).
    pub name: String,
    /// IPv4-Adresse mit Port, z. B. „192.168.0.20:47800“.
    pub address: String,
    /// Zeitpunkt der letzten Verbindung (Sekunden seit 1970).
    pub last_used: u64,
}

/// So viele Einträge merkt sich IPTalk unter „Zuletzt verbunden“.
pub const MAX_RECENT: usize = 5;

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

    /// Zuletzt verbundene Gegenstellen, neueste zuerst.
    pub recent: Vec<RecentPeer>,
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
            recent: Vec::new(),
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
    /// Merkt sich eine Verbindung. Ein bestehender Eintrag mit gleicher Adresse oder
    /// gleichem Namen (IP kann sich per DHCP ändern) wird aktualisiert und nach vorne geholt.
    pub fn remember_peer(&mut self, name: &str, address: &str, now: u64) {
        let name = name.trim();
        self.recent
            .retain(|r| r.address != address && (name.is_empty() || !r.name.eq_ignore_ascii_case(name)));
        self.recent.insert(0, RecentPeer { name: name.to_string(), address: address.to_string(), last_used: now });
        self.recent.truncate(MAX_RECENT);
    }

    pub fn forget_peer(&mut self, address: &str) {
        self.recent.retain(|r| r.address != address);
    }

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
        // Für Tests: andere Konfigurationsdatei verwenden, ohne die echte anzufassen.
        if let Some(p) = std::env::var_os("IPTALK_CONFIG") {
            return Some(PathBuf::from(p));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_moves_to_front_and_limits() {
        let mut c = Config::default();
        for i in 0..7 {
            c.remember_peer(&format!("PC{i}"), &format!("192.168.0.{i}:47800"), i);
        }
        assert_eq!(c.recent.len(), MAX_RECENT);
        assert_eq!(c.recent[0].name, "PC6");
        c.remember_peer("PC3", "192.168.0.3:47800", 100);
        assert_eq!(c.recent[0].name, "PC3");
        assert_eq!(c.recent.iter().filter(|r| r.name == "PC3").count(), 1);
    }

    #[test]
    fn same_name_new_ip_replaces_entry() {
        let mut c = Config::default();
        c.remember_peer("Linux-PC", "192.168.0.20:47800", 1);
        c.remember_peer("linux-pc", "192.168.0.35:47800", 2);
        assert_eq!(c.recent.len(), 1);
        assert_eq!(c.recent[0].address, "192.168.0.35:47800");
    }

    #[test]
    fn toml_roundtrip_with_tables() {
        let mut c = Config::default();
        c.ptt_key = Some(PttKey { code: 5, label: "Maustaste 4".into() });
        c.remember_peer("Linux-PC", "192.168.0.20:47800", 42);
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.recent, c.recent);
        assert_eq!(back.ptt_key, c.ptt_key);
        assert!(back.check_updates);
    }
}
