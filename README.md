<p align="center"><img src="assets/iptalk.png" width="128" alt="IPTalk-Icon"></p>

# IPTalk

**Sprachverbindung mit minimaler Latenz – direkt von Rechner zu Rechner im lokalen Netzwerk.**

IPTalk verbindet zwei Rechner im selben Netzwerk für Sprachkommunikation mit möglichst geringer Verzögerung. Gedacht ist es für Situationen, in denen sich zwei Personen am selben Ort mit geschlossenen Kopfhörern nicht direkt hören, etwa beim gemeinsamen Spielen. Herkömmliche Sprachdienste wie Discord sind dafür zu träge.

IPTalk läuft unter **Windows** und **Linux**. Es braucht keinen Server, kein Benutzerkonto und keine Installation.

| | typische Latenz (Mund → Ohr) |
|---|---|
| Discord und vergleichbare Dienste | 60 – 150 ms |
| IPTalk (LAN-Kabel, Onboard-Soundkarte, Windows) | **≈ 15 – 17 ms** (gemessen) |
| IPTalk (USB-Audiogerät bzw. Linux mit kleinem Quantum) | ≈ 8 – 12 ms |

Verzögerungen unter etwa 20 ms werden im Gespräch praktisch nicht mehr wahrgenommen.

---

## Inhalt

1. [Schnellstart](#1-schnellstart)
2. [Windows](#2-windows)
3. [Linux](#3-linux)
4. [Bedienung](#4-bedienung)
5. [Updates](#5-updates)
6. [Latenz weiter reduzieren](#6-latenz-weiter-reduzieren)
7. [Fehlerbehebung](#7-fehlerbehebung)
8. [Aus dem Quellcode bauen](#8-aus-dem-quellcode-bauen)
9. [Technischer Hintergrund](#9-technischer-hintergrund)

---

## 1. Schnellstart

1. Laden Sie unter **[Releases](https://github.com/Raphox2001/IPTalk/releases/latest)** die passende Datei herunter:
   - Windows: `IPTalk.exe`
   - Linux: `IPTalk-Linux`

   Die zusätzlich aufgeführten Dateien `iptalk-windows-x64.exe` und `iptalk-linux-x64` sind identisch und dienen nur der automatischen Aktualisierung älterer Versionen.
2. Starten Sie IPTalk auf **beiden** Rechnern.
3. Nach wenigen Sekunden erscheint der andere Rechner unter **„Im lokalen Netzwerk gefunden“**. Ein Klick darauf stellt die Verbindung her. Alternativ geben Sie die IP-Adresse der Gegenstelle ein und klicken auf **Verbinden**. Diese Adresse zeigt IPTalk auf dem anderen Rechner unter **„Eigene IP-Adresse“** an.
4. Die Gegenstelle nimmt die Verbindung automatisch an.

---

## 2. Windows

### Start
Legen Sie `IPTalk.exe` in einen Ordner Ihrer Wahl (z. B. `Dokumente\IPTalk`) und starten Sie die Datei per Doppelklick. Eine Installation ist nicht erforderlich. Die Einstellungen werden in `%APPDATA%\IPTalk\config.toml` gespeichert.

> Beim ersten Start zeigt Windows SmartScreen unter Umständen den Hinweis „Der Computer wurde durch Windows geschützt“. Wählen Sie dann **Weitere Informationen → Trotzdem ausführen**. Der Hinweis erscheint, weil die Datei nicht digital signiert ist.

### Firewall
Beim ersten Start fragt die Windows-Firewall nach einer Freigabe. Aktivieren Sie **„Private Netzwerke“** und bestätigen Sie mit **Zugriff zulassen**.

Falls die Abfrage nicht erschienen ist, legen Sie die Regel in einer **PowerShell mit Administratorrechten** an:

```powershell
New-NetFirewallRule -DisplayName "IPTalk" -Direction Inbound -Protocol UDP -LocalPort 47800 -Action Allow -Profile Private
```

Das Netzwerk muss in Windows außerdem als **privat** eingestuft sein: *Einstellungen → Netzwerk und Internet → Ethernet → Netzwerkprofiltyp: Privates Netzwerk*.

### Abtastrate 48 kHz (empfohlen)
IPTalk arbeitet intern mit 48 kHz. Ist ein Gerät in Windows auf 44,1 kHz eingestellt, muss zusätzlich umgerechnet werden, was etwas Latenz kostet. Stellen Sie das Format so um: *Systemsteuerung → Sound → Aufnahme → Mikrofon → Eigenschaften → Erweitert → „2 Kanal, 24 Bit, 48000 Hz“*. Für das Wiedergabegerät gehen Sie im Register *Wiedergabe* genauso vor.

---

## 3. Linux

IPTalk wird als einzelne ausführbare Datei ausgeliefert. Die Audiosysteme PulseAudio, PipeWire, JACK und ALSA werden erst zur Laufzeit geladen.

### Start
Legen Sie die Datei in einen dauerhaften Ordner, machen Sie sie ausführbar und starten Sie sie:
```bash
mkdir -p ~/Programme && mv ~/Downloads/IPTalk-Linux ~/Programme/
```
```bash
chmod +x ~/Programme/IPTalk-Linux
```
```bash
~/Programme/IPTalk-Linux
```

### Eintrag im Startmenü
Damit IPTalk mit Icon im Anwendungsmenü erscheint, klicken Sie in der Fußzeile des Fensters auf **„Ins Startmenü aufnehmen“**, oder führen Sie diesen Befehl aus:
```bash
~/Programme/IPTalk-Linux --install-desktop
```
Der Eintrag wird nur für den aktuellen Benutzer angelegt (`~/.local/share/applications/iptalk.desktop`) und verweist auf den aktuellen Speicherort der Datei. Wird die Datei verschoben, muss der Eintrag neu angelegt werden.

Eine systemweite Installation (z. B. nach `/usr/local/bin`) ist möglich. Dann kann sich IPTalk jedoch nicht selbst aktualisieren (siehe [Updates](#5-updates)).

Die Einstellungen werden in `~/.config/iptalk/config.toml` gespeichert.

### Benötigte Bibliotheken
Auf üblichen Desktop-Systemen sind alle Bibliotheken bereits vorhanden. Falls IPTalk nicht startet, installieren Sie diese Pakete:

| Distribution | Befehl |
|---|---|
| Ubuntu / Debian / Linux Mint / Pop!_OS | `sudo apt install libpulse0 libasound2 libgl1 libxkbcommon0` |
| Fedora | `sudo dnf install pulseaudio-libs alsa-lib mesa-libGL libxkbcommon` |
| Arch / Manjaro / CachyOS | `sudo pacman -S libpulse alsa-lib libglvnd libxkbcommon` |

Ob PipeWire aktiv ist, prüfen Sie mit `pactl info | grep "Server Name"`. Die Ausgabe sollte etwa „PulseAudio (on PipeWire …)“ lauten.

### Push-to-Talk unter Linux
Damit die Push-to-Talk-Taste auch dann erkannt wird, wenn ein anderes Programm im Vordergrund ist, liest IPTalk Tastatur und Maus direkt aus `/dev/input`. Das funktioniert unter X11 und Wayland. Dafür muss der Benutzer einmalig zur Gruppe `input` hinzugefügt werden:

```bash
sudo usermod -aG input $USER
```

Danach müssen Sie sich ab- und wieder anmelden. Ohne diese Berechtigung stehen alle anderen Sprechmodi weiterhin zur Verfügung.

### Firewall
Auf den meisten Desktop-Distributionen ist keine Firewall aktiv. Falls doch:

```bash
# Ubuntu (ufw)
sudo ufw allow 47800/udp
```

```bash
# Fedora (firewalld)
sudo firewall-cmd --permanent --add-port=47800/udp
```

```bash
sudo firewall-cmd --reload
```

### Geringste Latenz: JACK-Schnittstelle von PipeWire
Wählen Sie unter *Audio-Einstellungen → Audio-System* den Eintrag **„JACK / PipeWire-JACK“**. Dafür muss das passende Paket installiert sein:

| Distribution | Paket |
|---|---|
| Ubuntu / Debian | `pipewire-jack` |
| Fedora | `pipewire-jack-audio-connection-kit` |
| Arch | `pipewire-jack` |

IPTalk setzt beim Start die Variable `PIPEWIRE_LATENCY` (im Profil „Ultra“ z. B. `64/48000`). Mit JACK verwendet IPTalk die Standardgeräte von PipeWire. Diese ändern Sie mit `pavucontrol`, `qpwgraph` oder in den Systemeinstellungen.

Damit der Audio-Thread Echtzeitpriorität erhält, sollte der Dienst **rtkit** laufen. Bei den meisten Distributionen ist das Standard:

```bash
systemctl status rtkit-daemon
```

---

## 4. Bedienung

### Verbindung
- **Eigene IP-Adresse**: Die Adresse, unter der dieser Rechner im lokalen Netzwerk erreichbar ist. Diese Adresse gibt die Gegenstelle zum Verbinden ein. Über **Kopieren** landet sie in der Zwischenablage. Adressen von VPN-, WSL- und anderen virtuellen Adaptern werden nachrangig unter „Weitere Adressen“ aufgeführt.
- **IP-Adresse der Gegenstelle** eingeben und auf **Verbinden** klicken, oder einen Eintrag unter **„Im lokalen Netzwerk gefunden“** auswählen.
- **Eingehende Verbindungen automatisch annehmen**: Ist dies aktiviert, genügt es, wenn eine Seite die Verbindung herstellt.
- **Testmodus**: Die eigene Stimme durchläuft den vollständigen Netzwerkpfad und wird lokal zurückgespielt. So lassen sich Geräte, Rauschsperre und Pegel ohne Gegenstelle einrichten.
- Port: **47800/UDP**. Ändern lässt er sich mit `--port` oder in der `config.toml`.

### Sprechmodi
| Modus | Einsatzzweck |
|---|---|
| **Immer offen** | Ruhige Umgebung, gutes Mikrofon |
| **Rauschsperre** | Standard. Gesendet wird nur, solange gesprochen wird. Stellen Sie die Schwelle (gelbe Markierung in der Pegelanzeige) knapp oberhalb des Grundrauschens ein. |
| **Push-to-Talk** | Gesendet wird nur bei gedrückter Taste. Klicken Sie auf **„Taste festlegen“** und drücken Sie anschließend die gewünschte Taste oder Maustaste. Die Taste wirkt global, also auch in anderen Anwendungen. |

Zusätzlich gibt es Regler für **Mikrofon-Verstärkung** und **Lautstärke Gegenstelle** sowie je eine Stummschaltung.

### Latenzanzeige
- **Senderichtung** bzw. **Empfangsrichtung**: Die tatsächliche Verzögerung in beiden Richtungen, **laufend gemessen**. Gemessen wird vom Aufnahme-Callback über das Netzwerk und den Jitterpuffer bis zum Wiedergabe-Callback der Gegenseite, zuzüglich der Gerätepuffer. Grün steht für höchstens 12 ms, gelb für höchstens 25 ms.
- **Jitterpuffer**: Er gleicht Schwankungen im Netzwerk und die geringfügig unterschiedlichen Takte der beiden Soundkarten aus. Im Automatikmodus stellt er sich auf den kleinstmöglichen Wert ein.
- **Unterläufe**: Kurze Aussetzer in der Wiedergabe. Treten sie häufig auf, wählen Sie das Profil „Stabil“.

### Audio-Einstellungen
- **Mikrofon / Kopfhörer**: Wahl der Geräte.
- **Latenz-Profil**:
  - **Ultra** (Standard): kleinste Puffer (64 Frames angefordert).
  - **Stabil**: 256 Frames, für Systeme mit Aussetzern.
  - **Manuell**: frei wählbare Puffergröße. Unter Windows zusätzlich der *WASAPI-Exklusivmodus* für das Mikrofon. IPTalk prüft dabei automatisch, ob der Treiber korrekt getaktete Daten liefert, und wechselt andernfalls in den geteilten Modus.
- Der **Kopfhörer im Exklusivmodus** ist möglich. Andere Anwendungen sind dann jedoch auf diesem Gerät nicht mehr hörbar.

---

## 5. Updates

IPTalk prüft beim Start, ob auf GitHub eine neuere Version veröffentlicht wurde. Ist das der Fall, erscheint oben im Fenster ein Hinweis:

1. **Jetzt aktualisieren** lädt die neue Version herunter und ersetzt die vorhandene Programmdatei.
2. **Jetzt neu starten** beendet IPTalk und startet die neue Version. Eine bestehende Verbindung wird dabei kurz unterbrochen. Die Einstellungen bleiben erhalten.

In der Fußzeile lässt sich die Suche mit **„Nach Updates suchen“** jederzeit von Hand auslösen. Mit **„Beim Start prüfen“** schalten Sie die automatische Prüfung ab oder wieder ein.

Über die Kommandozeile funktioniert das genauso:
```bash
iptalk --update
```

**Voraussetzung:** Der Benutzer braucht Schreibrechte im Ordner der Programmdatei. Liegt IPTalk in einem geschützten Verzeichnis (z. B. `C:\Program Files` oder `/usr/local/bin`), meldet IPTalk das. Laden Sie die neue Version dann manuell von der [Release-Seite](https://github.com/Raphox2001/IPTalk/releases/latest).

---

## 6. Latenz weiter reduzieren

1. **Kabelverbindung statt WLAN.** WLAN schwankt stärker und erfordert einen größeren Jitterpuffer.
2. **Keine Bluetooth-Kopfhörer.** Bluetooth allein verursacht 100 bis 200 ms Verzögerung.
3. **Windows:** Mikrofon und Kopfhörer auf 48 kHz einstellen (siehe oben).
4. **Profil „Ultra“** verwenden. Bei Aussetzern unter „Manuell“ 128 Frames versuchen oder „Stabil“ wählen.
5. **Linux:** Als Audio-System **„JACK / PipeWire-JACK“** wählen.
6. **USB-Audiogeräte** lassen oft kleinere Puffer zu als Onboard-Soundkarten.
7. Unter *Audio-Einstellungen* zeigt IPTalk die tatsächlich ausgehandelten Puffer an, z. B. „Mikrofon: 139 Frames × 2 @ 48000 Hz = 2.9 ms“.

---

## 7. Fehlerbehebung

| Problem | Lösung |
|---|---|
| **„Keine Antwort von …“** | Prüfen Sie, ob IPTalk auf der Gegenstelle läuft, ob die Firewall-Regel besteht (siehe [Windows](#firewall) / [Linux](#firewall-1)) und ob beide Rechner im selben Netzwerk sind. |
| Gegenstelle erscheint nicht unter „Im lokalen Netzwerk gefunden“ | Manche Router blockieren Broadcasts. Geben Sie dann die IP-Adresse von Hand ein. |
| Keine Audioübertragung | Gerätewahl prüfen und den Testmodus verwenden. Beobachten Sie die Pegelanzeige unter „Sprechen“. Bei Rauschsperre ist die Schwelle möglicherweise zu hoch. |
| Knacken oder Aussetzer | Profil „Stabil“ wählen, oder unter „Manuell“ 128 bzw. 256 Frames einstellen. Im WLAN den Jitterpuffer manuell auf 5 bis 10 ms setzen. |
| Push-to-Talk reagiert nicht (Linux) | `sudo usermod -aG input $USER` ausführen und neu anmelden. Mit `groups` prüfen, ob `input` aufgeführt ist. |
| Push-to-Talk reagiert nicht (Windows) | Läuft die andere Anwendung mit Administratorrechten, muss auch IPTalk mit Administratorrechten laufen. |
| „Port 47800 konnte nicht geöffnet werden“ | IPTalk läuft bereits. Alternativ mit `--port 47801` starten und auf der Gegenstelle `IP:47801` angeben. |
| Andere Anwendungen auf dem Kopfhörer stumm | Unter „Manuell“ die Option „Kopfhörer exklusiv“ deaktivieren. |
| Linux: Fenster öffnet sich nicht | Bibliotheken aus [Abschnitt 3](#benötigte-bibliotheken) installieren und IPTalk im Terminal starten, um die Fehlermeldung zu sehen. |

### Diagnose über die Kommandozeile
```bash
iptalk --list-devices
```
Listet alle verfügbaren Audiosysteme und Geräte auf.

```bash
iptalk --selftest
```
Öffnet die Geräte ohne Fenster, spielt die eigene Stimme über 127.0.0.1 zurück und gibt jede Sekunde Latenz, Pufferstand und Aussetzer aus. Die Wiedergabe ist dabei stummgeschaltet, damit es über Lautsprecher keine Rückkopplung gibt.

```bash
iptalk --selftest --connect 192.168.1.20 --seconds 30
```
Führt denselben Test gegen eine Gegenstelle aus, auf der IPTalk laufen muss.

Alle Optionen zeigt `iptalk --help`.

---

## 8. Aus dem Quellcode bauen

Voraussetzung ist **Rust** (stabile Version), das Sie über [rustup.rs](https://rustup.rs) installieren.

### Windows
1. Rust über [rustup-init.exe](https://rustup.rs) installieren.
2. Die **Visual Studio Build Tools** mit der Komponente „Desktopentwicklung mit C++“ installieren. rustup bietet das an.
3. Repository klonen und bauen:
   ```powershell
   git clone https://github.com/Raphox2001/IPTalk.git
   cd IPTalk
   cargo build --release
   ```
4. Die Programmdatei liegt danach unter `target\release\iptalk.exe`.

### Linux
1. Compiler und Werkzeuge installieren. Unter Ubuntu/Debian:
   ```bash
   sudo apt install build-essential git curl
   ```
   Unter Fedora heißt das Paket `gcc`, unter Arch `base-devel`.
2. Rust installieren:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
3. Repository klonen:
   ```bash
   git clone https://github.com/Raphox2001/IPTalk.git
   ```
4. Bauen:
   ```bash
   cd IPTalk && cargo build --release
   ```
5. Die Programmdatei liegt danach unter `target/release/iptalk`.

Entwicklerpakete wie `libpulse-dev` oder `libasound2-dev` sind **nicht** erforderlich. Die Tests starten Sie mit `cargo test --release`.

### Veröffentlichung
GitHub Actions baut bei jedem Push beide Versionen. Ein Tag der Form `v*` (z. B. `v0.3.0`) erzeugt automatisch ein Release mit beiden Programmdateien, das die Update-Funktion dann anbietet. Die Versionsnummer in `Cargo.toml` muss zum Tag passen.

---

## 9. Technischer Hintergrund

Die gesamte Signalkette ist auf möglichst geringe Verzögerung ausgelegt:

- **Kein Codec.** Audio wird unkomprimiert übertragen (48 kHz, mono, 16 Bit, also 768 kbit/s). Für ein lokales Netzwerk ist diese Datenrate unerheblich. Ein Codec wie Opus würde 5 bis 25 ms für Framing und Lookahead hinzufügen.
- **UDP direkt zwischen den Rechnern**, ohne Server und ohne TCP-Retransmissionen. Jeder Audio-Callback wird sofort als eigenes Paket versendet (höchstens 2,5 ms Audio pro Paket).
- **miniaudio** als Audioschicht:
  - Windows: WASAPI im Low-Latency-Modus (IAudioClient3, MMCSS „Pro Audio“), optional Exclusive.
  - Linux: PulseAudio/PipeWire, JACK oder ALSA mit kleinem Quantum.
- **Adaptiver Jitterpuffer mit Driftausgleich.** Zwei Soundkarten laufen nie exakt gleich schnell; ±100 ppm sind üblich und würden ohne Ausgleich zu stetig wachsender Verzögerung führen. Ein PI-Regler hält den minimalen Füllstand des Puffers auf einer kleinen Reserve. Bei Bedarf lässt er pro Audioblock ein Sample aus oder fügt eines ein, durch Interpolation über den Block praktisch unhörbar. Treten Aussetzer auf, vergrößert sich die Reserve automatisch und baut sich bei stabilem Betrieb wieder ab.
- **Echte Latenzmessung.** Der Empfänger meldet zehnmal pro Sekunde, welches Sample (mit dem Zeitstempel des Senders) gerade abgespielt wird. Daraus ergibt sich ohne Uhrensynchronisation die tatsächliche Verzögerung.

### Projektstruktur
```
native/miniaudio.h   Audiobibliothek (Single Header, v0.11.25)
native/ma_shim.c     schlanke C-Schnittstelle für Rust
src/audio.rs         sichere Rust-Kapselung von miniaudio
src/protocol.rs      UDP-Paketformat
src/jitter.rs        Jitterpuffer und Driftregler (mit Simulationstests)
src/dsp.rs           Verstärkung, Rauschsperre, Push-to-Talk-Logik
src/engine.rs        Netzwerk-Threads, Verbindungsaufbau, Audiogeräte
src/ptt.rs           globale Push-to-Talk-Taste (Windows: GetAsyncKeyState, Linux: evdev)
src/update.rs        Aktualisierung über GitHub-Releases
src/netinfo.rs       Ermittlung der eigenen IP-Adressen
src/desktop.rs       Fenster-Icon, Startmenü-Eintrag unter Linux
assets/              Icon (make_icon.py erzeugt iptalk.ico/.png)
src/ui.rs            Benutzeroberfläche (egui)
src/main.rs          Start, Kommandozeilenoptionen, Selbsttest
```

### Protokoll
UDP-Port 47800. Jedes Paket beginnt mit einem 20-Byte-Header: `IPTK`, Version, Typ, Flags, Sequenznummer und Zeitstempel in µs. Paketarten:
- Audio
- Hello/HelloAck (Verbindungsaufbau)
- Discover/Announce (Netzwerksuche per Broadcast)
- Ping/Pong
- Played (Latenzmessung)
- Stats
- Bye

**Hinweis:** Die Übertragung ist **nicht verschlüsselt** und für den Einsatz im eigenen lokalen Netzwerk vorgesehen.

## Lizenz
MIT, siehe [LICENSE](LICENSE). miniaudio steht unter Public Domain bzw. MIT-0.
