//! Aktualisierung über GitHub-Releases.
//!
//! Ablauf: neuestes Release abfragen → passende Datei herunterladen → laufendes
//! Programm ersetzen → nach dem Beenden neu starten. Unter Windows lässt sich eine
//! laufende .exe nicht überschreiben, aber umbenennen; die alte Datei wird beim
//! nächsten Start aufgeräumt.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "Raphox2001/IPTalk";
pub const RELEASES_URL: &str = "https://github.com/Raphox2001/IPTalk/releases";

#[cfg(windows)]
const ASSET_NAME: &str = "iptalk-windows-x64.exe";
#[cfg(not(windows))]
const ASSET_NAME: &str = "iptalk-linux-x64";

/// Wird gesetzt, wenn nach dem Schließen des Fensters die neue Version gestartet werden soll.
pub static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    download_url: String,
    size: u64,
}

#[derive(Clone, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { version: String, done: u64, total: u64 },
    /// Neue Version liegt bereit, Neustart ausstehend.
    Installed(String),
    Failed(String),
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<GhAsset>,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Version des laufenden Programms. `IPTALK_FAKE_VERSION` dient zum Testen des Updates.
pub fn current_version() -> String {
    std::env::var("IPTALK_FAKE_VERSION").unwrap_or_else(|_| CURRENT_VERSION.to_string())
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// `true`, wenn `candidate` neuer ist als `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(&format!("IPTalk/{CURRENT_VERSION}"))
        .build()
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(404, _) => "Keine veröffentlichte Version gefunden.".into(),
        ureq::Error::Status(403, _) => "GitHub-Anfragelimit erreicht. Bitte später erneut versuchen.".into(),
        ureq::Error::Status(code, _) => format!("GitHub antwortet mit Status {code}."),
        ureq::Error::Transport(t) => format!("Keine Verbindung zu GitHub ({t})."),
    }
}

/// Fragt das neueste Release ab. `Ok(None)` = bereits aktuell.
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let rel: GhRelease = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(describe)?
        .into_json()
        .map_err(|e| format!("Antwort von GitHub nicht lesbar: {e}"))?;
    if rel.draft || rel.prerelease || !is_newer(&rel.tag_name, &current_version()) {
        return Ok(None);
    }
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name == ASSET_NAME)
        .ok_or_else(|| format!("Version {} enthält keine Datei für dieses System ({ASSET_NAME}).", rel.tag_name))?;
    Ok(Some(Release {
        version: rel.tag_name.trim_start_matches(['v', 'V']).to_string(),
        page_url: rel.html_url,
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
    }))
}

fn exe_path() -> Result<PathBuf, String> {
    let p = std::env::current_exe().map_err(|e| format!("Programmpfad nicht ermittelbar: {e}"))?;
    Ok(std::fs::canonicalize(&p).unwrap_or(p))
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

fn new_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".new");
    exe.with_file_name(name)
}

/// Entfernt Reste eines vorherigen Updates (Windows: die umbenannte alte .exe).
pub fn cleanup_previous() {
    if let Ok(exe) = exe_path() {
        let _ = std::fs::remove_file(old_path(&exe));
        let _ = std::fs::remove_file(new_path(&exe));
    }
}

/// Lädt die neue Version herunter und ersetzt das laufende Programm.
/// `progress(geladen, gesamt)` wird während des Downloads aufgerufen.
pub fn install(rel: &Release, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let exe = exe_path()?;
    let tmp = new_path(&exe);
    let no_write = |e: std::io::Error| {
        format!(
            "Keine Schreibrechte im Programmordner {} ({e}). Bitte die neue Version manuell herunterladen.",
            exe.parent().map(|p| p.display().to_string()).unwrap_or_default()
        )
    };

    let resp = agent().get(&rel.download_url).call().map_err(describe)?;
    let total = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(rel.size);
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tmp).map_err(no_write)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download abgebrochen: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("Schreiben fehlgeschlagen: {e}"))?;
        done += n as u64;
        progress(done, total);
    }
    file.sync_all().map_err(|e| format!("Schreiben fehlgeschlagen: {e}"))?;
    drop(file);

    if done != rel.size {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Download unvollständig ({done} von {} Bytes).", rel.size));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(no_write)?;
    }
    #[cfg(windows)]
    {
        // Laufende .exe beiseite schieben, dann die neue an ihre Stelle setzen.
        let old = old_path(&exe);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(no_write)?;
        if let Err(e) = std::fs::rename(&tmp, &exe) {
            let _ = std::fs::rename(&old, &exe);
            return Err(no_write(e));
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(&tmp, &exe).map_err(no_write)?;
    Ok(())
}

/// Startet das (neue) Programm mit denselben Argumenten.
pub fn relaunch() {
    if let Ok(exe) = exe_path() {
        let _ = std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn();
    }
}

/// Hält den Update-Zustand für die Oberfläche und führt Prüfen/Installieren im Hintergrund aus.
pub struct Updater {
    pub state: Arc<Mutex<State>>,
}

impl Updater {
    pub fn new() -> Updater {
        Updater { state: Arc::new(Mutex::new(State::Idle)) }
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Checking | State::Downloading { .. })
    }

    pub fn check_in_background(&self) {
        if self.busy() {
            return;
        }
        *self.state.lock().unwrap() = State::Checking;
        let st = self.state.clone();
        std::thread::spawn(move || {
            let next = match check() {
                Ok(Some(r)) => State::Available(r),
                Ok(None) => State::UpToDate,
                Err(e) => State::Failed(e),
            };
            *st.lock().unwrap() = next;
        });
    }

    pub fn install_in_background(&self, rel: Release) {
        if self.busy() {
            return;
        }
        let version = rel.version.clone();
        *self.state.lock().unwrap() = State::Downloading { version: version.clone(), done: 0, total: rel.size };
        let st = self.state.clone();
        std::thread::spawn(move || {
            let st2 = st.clone();
            let v2 = version.clone();
            let result = install(&rel, move |done, total| {
                *st2.lock().unwrap() = State::Downloading { version: v2.clone(), done, total };
            });
            *st.lock().unwrap() = match result {
                Ok(()) => State::Installed(version),
                Err(e) => State::Failed(e),
            };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.2.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("v0.2.0-beta", "0.2.0"));
    }

    #[test]
    fn helper_paths() {
        let exe = Path::new("/opt/iptalk/iptalk.exe");
        assert_eq!(old_path(exe), Path::new("/opt/iptalk/iptalk.exe.old"));
        assert_eq!(new_path(exe), Path::new("/opt/iptalk/iptalk.exe.new"));
    }
}
