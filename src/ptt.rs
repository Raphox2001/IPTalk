//! Globale Push-to-Talk-Taste (funktioniert auch, wenn das Spiel im Vordergrund ist).
//!
//! Windows: `GetAsyncKeyState` wird alle 5 ms abgefragt.
//! Linux: Tastatur- und Maus-Ereignisse direkt aus `/dev/input` (X11 und Wayland).
//! Dafür muss der Benutzer in der Gruppe `input` sein.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::*};
use std::sync::{Arc, Mutex};

use crate::config::PttKey;

pub struct Ptt {
    /// Aktuell gedrückt? (wird vom Capture-Callback gelesen)
    pub pressed: AtomicBool,
    /// Gewählte Taste, 0 = keine.
    key: AtomicU32,
    /// Wartet auf einen Tastendruck zum Zuweisen.
    learning: AtomicBool,
    learned: Mutex<Option<PttKey>>,
    /// Hinweis für die Oberfläche (z. B. fehlende Berechtigung unter Linux).
    pub problem: Mutex<Option<String>>,
}

impl Ptt {
    pub fn start(key: Option<&PttKey>) -> Arc<Ptt> {
        let ptt = Arc::new(Ptt {
            pressed: AtomicBool::new(false),
            key: AtomicU32::new(key.map(|k| k.code).unwrap_or(0)),
            learning: AtomicBool::new(false),
            learned: Mutex::new(None),
            problem: Mutex::new(None),
        });
        backend::spawn(ptt.clone());
        ptt
    }

    pub fn start_learning(&self) {
        *self.learned.lock().unwrap() = None;
        self.learning.store(true, Relaxed);
    }

    pub fn cancel_learning(&self) {
        self.learning.store(false, Relaxed);
    }

    pub fn is_learning(&self) -> bool {
        self.learning.load(Relaxed)
    }

    /// Liefert die neu zugewiesene Taste, sobald der Benutzer eine gedrückt hat.
    pub fn take_learned(&self) -> Option<PttKey> {
        self.learned.lock().unwrap().take()
    }

    fn on_learned(&self, key: PttKey) {
        self.key.store(key.code, Relaxed);
        *self.learned.lock().unwrap() = Some(key);
        self.learning.store(false, Relaxed);
    }
}

#[cfg(windows)]
mod backend {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetKeyNameTextW, MapVirtualKeyW, MAPVK_VK_TO_VSC,
    };

    fn down(vk: u32) -> bool {
        (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
    }

    fn label(vk: u32) -> String {
        match vk {
            0x02 => return "Rechte Maustaste".into(),
            0x04 => return "Mittlere Maustaste".into(),
            0x05 => return "Maustaste 4 (Seite hinten)".into(),
            0x06 => return "Maustaste 5 (Seite vorne)".into(),
            0xA0 => return "Shift links".into(),
            0xA1 => return "Shift rechts".into(),
            0xA2 => return "Strg links".into(),
            0xA3 => return "Strg rechts".into(),
            0xA4 => return "Alt links".into(),
            0xA5 => return "Alt Gr".into(),
            0x14 => return "Feststelltaste".into(),
            _ => {}
        }
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) };
        let mut buf = [0u16; 64];
        let n = unsafe { GetKeyNameTextW((scan << 16) as i32, buf.as_mut_ptr(), buf.len() as i32) };
        if n > 0 {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            format!("Taste 0x{vk:02X}")
        }
    }

    pub fn spawn(ptt: Arc<Ptt>) {
        std::thread::Builder::new()
            .name("ptt".into())
            .spawn(move || {
                let mut prev = [false; 256];
                loop {
                    if ptt.learning.load(Relaxed) {
                        for vk in 2u32..255 {
                            // Linke Maustaste (Klick auf den Knopf) und die generischen
                            // Shift/Strg/Alt-Codes überspringen, die L/R-Varianten kommen extra.
                            if (0x10..=0x12).contains(&vk) {
                                continue;
                            }
                            let d = down(vk);
                            if d && !prev[vk as usize] {
                                ptt.on_learned(PttKey { code: vk, label: label(vk) });
                            }
                            prev[vk as usize] = d;
                        }
                    } else {
                        prev = [true; 256]; // beim nächsten Lernen erst auf neue Drücke reagieren
                        let key = ptt.key.load(Relaxed);
                        ptt.pressed.store(key != 0 && down(key), Relaxed);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            })
            .expect("PTT-Thread");
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use super::*;
    use evdev::{EventType, Key};

    const BTN_LEFT: u16 = 0x110;

    fn label(code: u16) -> String {
        match code {
            0x111 => "Rechte Maustaste".into(),
            0x112 => "Mittlere Maustaste".into(),
            0x113 => "Maustaste Seite (hinten)".into(),
            0x114 => "Maustaste Extra (vorne)".into(),
            0x115 => "Maustaste vorwärts".into(),
            0x116 => "Maustaste zurück".into(),
            _ => {
                let s = format!("{:?}", Key::new(code));
                s.strip_prefix("KEY_").unwrap_or(&s).to_string()
            }
        }
    }

    pub fn spawn(ptt: Arc<Ptt>) {
        let mut count = 0;
        for (_, mut dev) in evdev::enumerate() {
            let has_keys = dev.supported_keys().map(|k| k.iter().next().is_some()).unwrap_or(false);
            if !has_keys {
                continue;
            }
            count += 1;
            let ptt = ptt.clone();
            let _ = std::thread::Builder::new().name("ptt-evdev".into()).spawn(move || loop {
                let events = match dev.fetch_events() {
                    Ok(ev) => ev,
                    Err(_) => return, // Gerät abgesteckt
                };
                for ev in events {
                    if ev.event_type() != EventType::KEY {
                        continue;
                    }
                    let code = ev.code();
                    match ev.value() {
                        1 => {
                            if ptt.learning.load(Relaxed) {
                                if code != BTN_LEFT {
                                    ptt.on_learned(PttKey { code: code as u32, label: label(code) });
                                }
                            } else if code as u32 == ptt.key.load(Relaxed) {
                                ptt.pressed.store(true, Relaxed);
                            }
                        }
                        0 => {
                            if code as u32 == ptt.key.load(Relaxed) {
                                ptt.pressed.store(false, Relaxed);
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
        if count == 0 {
            *ptt.problem.lock().unwrap() = Some(
                "Kein Zugriff auf Tastatur/Maus (/dev/input). Für Push-to-Talk einmalig ausführen:\n\
                 sudo usermod -aG input $USER\nund danach ab- und wieder anmelden."
                    .into(),
            );
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod backend {
    use super::*;
    pub fn spawn(ptt: Arc<Ptt>) {
        *ptt.problem.lock().unwrap() = Some("Push-to-Talk wird auf diesem System nicht unterstützt.".into());
    }
}
