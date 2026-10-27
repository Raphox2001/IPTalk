//! Icon und Startmenü-Integration.

use eframe::egui;

/// Wayland-App-ID bzw. X11-Fensterklasse; muss zum Namen der .desktop-Datei passen.
pub const APP_ID: &str = "iptalk";

const ICON_RGBA: &[u8] = include_bytes!("../assets/iptalk-64.rgba");
#[cfg(target_os = "linux")]
const ICON_PNG: &[u8] = include_bytes!("../assets/iptalk.png");

/// Fenster-Icon (Titelleiste/Taskleiste).
pub fn window_icon() -> egui::IconData {
    egui::IconData { rgba: ICON_RGBA.to_vec(), width: 64, height: 64 }
}

/// Legt einen Startmenü-Eintrag mit Icon für den aktuellen Benutzer an
/// (`~/.local/share/applications/iptalk.desktop`). Gibt den Pfad der .desktop-Datei zurück.
#[cfg(target_os = "linux")]
pub fn install_desktop_entry() -> Result<std::path::PathBuf, String> {
    let data = dirs::data_dir().ok_or("Benutzerdatenordner nicht gefunden.")?;
    let exe = std::env::current_exe().map_err(|e| format!("Programmpfad nicht ermittelbar: {e}"))?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);

    let icon_dir = data.join("icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&icon_dir).map_err(|e| format!("{}: {e}", icon_dir.display()))?;
    std::fs::write(icon_dir.join(format!("{APP_ID}.png")), ICON_PNG).map_err(|e| format!("Icon: {e}"))?;

    let app_dir = data.join("applications");
    std::fs::create_dir_all(&app_dir).map_err(|e| format!("{}: {e}", app_dir.display()))?;
    let file = app_dir.join(format!("{APP_ID}.desktop"));
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=IPTalk\n\
         GenericName=Sprachverbindung\n\
         Comment=Sprachverbindung mit minimaler Latenz im lokalen Netzwerk\n\
         Exec=\"{}\"\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;Network;\n\
         StartupWMClass={APP_ID}\n",
        exe.display()
    );
    std::fs::write(&file, entry).map_err(|e| format!("{}: {e}", file.display()))?;
    // Menüs neu einlesen lassen (falls vorhanden; Fehler sind unkritisch).
    let _ = std::process::Command::new("update-desktop-database").arg(&app_dir).status();
    Ok(file)
}

/// Gibt es den Startmenü-Eintrag schon?
#[cfg(target_os = "linux")]
pub fn desktop_entry_exists() -> bool {
    dirs::data_dir().map(|d| d.join(format!("applications/{APP_ID}.desktop")).exists()).unwrap_or(false)
}
