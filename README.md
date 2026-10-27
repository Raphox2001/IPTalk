# IPTalk

**Sprachverbindung mit minimaler Latenz – direkt von PC zu PC im Heimnetz.**

IPTalk ist für genau einen Zweck gebaut: Zwei Leute sitzen nebeneinander, zocken mit Kopfhörern und wollen sich trotzdem hören, ohne die Verzögerung von Discord und ohne die vielen Optionen von SonoBus. IPTalk läuft unter **Windows** und **Linux**. Es braucht keinen Server und kein Konto, nur zwei PCs im selben Netzwerk.

| | typische Latenz (Mund → Ohr) |
|---|---|
| Discord | 60 – 150 ms |
| IPTalk (LAN-Kabel, Onboard-Soundkarte, Windows) | **≈ 15 – 17 ms** gemessen |
| IPTalk (gute USB-Soundkarte / Linux mit kleinem Quantum) | ≈ 8 – 12 ms |

Unter etwa 20 ms nimmt man die Verzögerung praktisch nicht mehr wahr.

---

## Inhalt

1. [Schnellstart](#1-schnellstart)
2. [Windows](#2-windows)
3. [Linux](#3-linux)
4. [Bedienung](#4-bedienung)
5. [So niedrig wie möglich: Latenz-Tipps](#5-so-niedrig-wie-möglich-latenz-tipps)
6. [Fehlerbehebung](#6-fehlerbehebung)
7. [Selbst bauen](#7-selbst-bauen)
8. [Wie es funktioniert](#8-wie-es-funktioniert)

---

## 1. Schnellstart

1. Lade unter **[Releases](../../releases)** die passende Datei herunter:
   - Windows: `iptalk-windows-x64.exe`
   - Linux: `iptalk-linux-x64`
2. Starte IPTalk auf **beiden** PCs.
3. Nach ein paar Sekunden erscheint der andere PC unter **„Im Netzwerk gefunden“**. Ein Klick darauf verbindet euch. Alternativ gibst du die IP des anderen ein und klickst auf **Verbinden**.
4. Fertig. Die Gegenseite nimmt automatisch an.

> Die IP-Adresse findest du unter Windows mit `ipconfig` (Zeile „IPv4-Adresse“) und unter Linux mit `ip -4 addr` oder `hostname -I`.

---

## 2. Windows

### Starten
`iptalk-windows-x64.exe` einfach per Doppelklick starten, eine Installation ist nicht nötig. Die Einstellungen landen in `%APPDATA%\IPTalk\config.toml`.

> Beim ersten Start warnt Windows SmartScreen eventuell vor einer „unbekannten App“. Klicke auf **Weitere Informationen → Trotzdem ausführen**. Das liegt nur daran, dass die Datei nicht signiert ist.

### Firewall
Beim ersten Start fragt die Windows-Firewall nach. Setze den Haken bei **„Private Netzwerke“** und klicke auf **Zugriff zulassen**.

Falls die Frage nicht kam oder du sie weggeklickt hast, kannst du die Regel in einer **PowerShell als Administrator** von Hand anlegen:

```powershell
New-NetFirewallRule -DisplayName "IPTalk" -Direction Inbound -Protocol UDP -LocalPort 47800 -Action Allow -Profile Private
```

Außerdem muss dein Netzwerk in Windows als **„Privat“** eingestuft sein: *Einstellungen → Netzwerk und Internet → Ethernet → Netzwerkprofiltyp: Privates Netzwerk*.

### Gerät auf 48 kHz stellen (empfohlen)
IPTalk arbeitet intern mit 48 kHz. Steht dein Headset unter Windows auf 44,1 kHz, muss zusätzlich umgerechnet werden. Das kostet etwas Latenz.
*Systemsteuerung → Sound → Aufnahme → Mikrofon → Eigenschaften → Erweitert → „2 Kanal, 24 Bit, 48000 Hz“*. Für den Kopfhörer im Tab *Wiedergabe* genauso.

---

## 3. Linux

IPTalk ist ein einzelnes Programm. PulseAudio, PipeWire, JACK und ALSA lädt es erst zur Laufzeit, deshalb braucht es keine Extra-Pakete zum Starten.

### Starten
```bash
chmod +x iptalk-linux-x64
./iptalk-linux-x64
```

Optional kannst du es systemweit installieren:
```bash
sudo install -m 755 iptalk-linux-x64 /usr/local/bin/iptalk
```

Die Einstellungen landen in `~/.config/iptalk/config.toml`.

### Benötigte Bibliotheken
Auf normalen Desktop-Systemen ist schon alles vorhanden. Falls IPTalk nicht startet:

| Distribution | Befehl |
|---|---|
| Ubuntu / Debian / Mint / Pop!_OS | `sudo apt install libpulse0 libasound2 libgl1 libxkbcommon0` |
| Fedora | `sudo dnf install pulseaudio-libs alsa-lib mesa-libGL libxkbcommon` |
| Arch / Manjaro / CachyOS | `sudo pacman -S libpulse alsa-lib libglvnd libxkbcommon` |

Ob PipeWire läuft, zeigt `pactl info | grep "Server Name"`. Die Ausgabe sollte etwa „PulseAudio (on PipeWire …)“ lauten. Aktuelle Distributionen nutzen fast alle PipeWire.

### Push-to-Talk unter Linux
Damit die PTT-Taste **auch im Spiel** (X11 und Wayland) erkannt wird, liest IPTalk Tastatur und Maus direkt aus `/dev/input`. Dafür musst du dich einmalig der Gruppe `input` hinzufügen:

```bash
sudo usermod -aG input $USER
```

Danach **ab- und wieder anmelden** oder neu starten. Ohne diese Berechtigung funktionieren alle anderen Sprechmodi trotzdem.

### Firewall
Die meisten Desktop-Distributionen haben keine aktive Firewall. Falls doch:

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

### Niedrigste Latenz unter Linux: JACK-Schnittstelle von PipeWire
Unter *Audio-Einstellungen → Audio-System* **„JACK / PipeWire-JACK“** wählen. Dafür muss das passende Paket installiert sein:

| Distribution | Paket |
|---|---|
| Ubuntu / Debian | `pipewire-jack` |
| Fedora | `pipewire-jack-audio-connection-kit` |
| Arch | `pipewire-jack` |

IPTalk setzt beim Start selbst `PIPEWIRE_LATENCY` (z. B. `64/48000` im Profil „Ultra“). Bei JACK wählt IPTalk die Geräte nicht selbst, sie folgen den Standardgeräten von PipeWire. Umstellen kannst du sie mit `pavucontrol`, `qpwgraph` oder den Systemeinstellungen.

Damit der Audio-Thread Echtzeit-Priorität bekommt, sollte **rtkit** laufen. Das ist bei den meisten Distributionen Standard:

```bash
systemctl status rtkit-daemon
```

---

## 4. Bedienung

### Verbindung
- **IP des Kumpels** eintragen und auf **Verbinden** klicken, oder einen Eintrag aus **„Im Netzwerk gefunden“** anklicken.
- **Anrufe automatisch annehmen**: Ruft dich jemand an, während du nicht verbunden bist, wird sofort verbunden. Es reicht also, wenn **einer** von euch auf Verbinden klickt.
- **Testmodus**: Deine eigene Stimme läuft über den kompletten Netzwerkweg zu dir zurück. Damit kannst du allein Geräte, Rauschsperre und Lautstärke einstellen.
- Port: **47800/UDP**. Ändern lässt er sich per `--port` oder in der `config.toml`.

### Sprechmodi
| Modus | Wann sinnvoll |
|---|---|
| **Immer offen** | Ruhige Umgebung, gutes Headset |
| **Rauschsperre** | Standard. Gesendet wird nur, wenn du sprichst. Stell die **gelbe Linie** in der Pegelanzeige knapp über das Grundrauschen (Lüfter, Tastatur). |
| **Push-to-Talk** | Klick auf **„Taste festlegen“**, dann die gewünschte Taste oder Maustaste drücken (z. B. Maus-Seitentaste). Sie funktioniert global, auch im Spiel. |

Dazu gibt es **Mikro-Verstärkung**, **Lautstärke Kumpel** und je einen **Stumm-Knopf**.

### Latenz-Anzeige
- **Deine Stimme beim Kumpel** / **Kumpels Stimme bei dir**: wird **live gemessen**. IPTalk misst die Zeit vom Mikrofon-Callback über das Netz und den Jitterpuffer bis zum Kopfhörer-Callback der Gegenseite und addiert die Gerätepuffer. Grün heißt bis 12 ms, gelb bis 25 ms.
- **Jitterpuffer**: Er gleicht Schwankungen im Netz und die minimal unterschiedlichen Takte der beiden Soundkarten aus. Im Auto-Modus stellt er sich selbst so klein wie möglich ein.
- **Unterläufe**: Aussetzer im Ton. Treten sie häufiger auf, im Profil „Stabil“ wählen oder die Puffergröße erhöhen.

### Audio-Einstellungen (aufklappen)
- **Mikrofon / Kopfhörer**: Geräte wählen.
- **Latenz-Profil**
  - **Ultra**: kleinste Puffer (64 Frames angefordert). Standard.
  - **Stabil**: 256 Frames, falls es knackst.
  - **Manuell**: Puffergröße selbst wählen. Unter Windows zusätzlich *WASAPI Exclusive* für das Mikrofon. IPTalk prüft dabei automatisch, ob der Treiber das korrekt kann, und fällt sonst auf den geteilten Modus zurück.
- Den **Kopfhörer exklusiv** zu öffnen ist möglich, aber dann ist der **Spielsound auf diesem Gerät stumm**. Nur nutzen, wenn das Spiel über ein anderes Gerät läuft.

---

## 5. So niedrig wie möglich: Latenz-Tipps

1. **LAN-Kabel statt WLAN.** WLAN schwankt stark, der Jitterpuffer muss dann größer werden.
2. **Keine Bluetooth-Kopfhörer.** Bluetooth allein kostet 100 bis 200 ms.
3. **Windows: Mikrofon und Kopfhörer auf 48 kHz stellen** (siehe oben).
4. **Profil „Ultra“** verwenden. Knackst es, auf „Stabil“ wechseln oder unter „Manuell“ 128 Frames probieren.
5. **Linux: „JACK / PipeWire-JACK“** als Audio-System wählen.
6. Eine **USB-Soundkarte oder ein USB-Headset** hat oft kleinere minimale Puffer als Onboard-Sound.
7. Im Fenster unter *Audio-Einstellungen* siehst du, welche Puffer tatsächlich ausgehandelt wurden, z. B. „Mikrofon: 139 Frames × 2 @ 48000 Hz = 2.9 ms“.

---

## 6. Fehlerbehebung

| Problem | Lösung |
|---|---|
| **„Keine Antwort von …“** | Läuft IPTalk auf dem anderen PC? Firewall-Regel prüfen (siehe [Windows](#firewall) / [Linux](#firewall-1)). Ist die IP richtig? Beide im selben Netz? |
| Der andere PC erscheint nicht unter „Im Netzwerk gefunden“ | Manche Router blockieren Broadcasts. Dann einfach die IP von Hand eingeben. |
| Kein Ton | Richtige Geräte gewählt? **Testmodus** probieren. Pegelanzeige unter „Sprechen“ ansehen. Bei Rauschsperre die Schwelle eventuell zu hoch. |
| Knacksen / Aussetzer | Profil „Stabil“ wählen, oder „Manuell“ mit 128 bzw. 256 Frames. Bei WLAN den Jitterpuffer auf manuell 5 bis 10 ms stellen. |
| Push-to-Talk reagiert nicht (Linux) | `sudo usermod -aG input $USER`, danach neu anmelden. Mit `groups` prüfen, ob `input` dabei ist. |
| Push-to-Talk reagiert nicht (Windows) | Läuft das Spiel als Administrator, muss IPTalk auch als Administrator laufen. |
| „Port 47800 konnte nicht geöffnet werden“ | IPTalk läuft schon (z. B. im Hintergrund). Sonst `--port 47801` verwenden und die Gegenseite mit `IP:47801` verbinden. |
| Spielsound weg | Unter „Manuell“ ist „Kopfhörer exklusiv“ aktiv. Ausschalten. |
| Linux: Fenster startet nicht | Bibliotheken aus [Abschnitt 3](#benötigte-bibliotheken) installieren. Im Terminal starten und die Fehlermeldung ansehen. |

### Diagnose über die Kommandozeile
```bash
iptalk --list-devices
```
listet alle Audio-Systeme und Geräte auf.

```bash
iptalk --selftest
```
öffnet die Geräte ohne Fenster, schickt die eigene Stimme über 127.0.0.1 zurück und gibt jede Sekunde Latenz, Puffer und Aussetzer aus. Die Ausgabe ist dabei stumm, damit es über Lautsprecher nicht rückkoppelt.

```bash
iptalk --selftest --connect 192.168.1.20 --seconds 30
```
macht dasselbe gegen den PC deines Kumpels. Dort muss IPTalk laufen.

Alle Optionen zeigt `iptalk --help`.

---

## 7. Selbst bauen

Voraussetzung ist **Rust** (stabile Version), das du über [rustup.rs](https://rustup.rs) installierst.

### Windows
1. Rust installieren: [rustup-init.exe](https://rustup.rs)
2. Die **Visual Studio Build Tools** mit „Desktopentwicklung mit C++“ installieren. rustup bietet das an.
3. Bauen:
   ```powershell
   git clone https://github.com/Raphox2001/IPTalk.git
   cd IPTalk
   cargo build --release
   ```
4. Das Programm liegt unter `target\release\iptalk.exe`.

### Linux
1. Compiler und Werkzeuge installieren (Ubuntu/Debian; bei Fedora `gcc`, bei Arch `base-devel`):
   ```bash
   sudo apt install build-essential git curl
   ```
2. Rust installieren:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
3. Repository holen und bauen:
   ```bash
   git clone https://github.com/Raphox2001/IPTalk.git
   ```
   ```bash
   cd IPTalk && cargo build --release
   ```
4. Das Programm liegt unter `target/release/iptalk`.

Header-Pakete wie `libpulse-dev` oder `libasound2-dev` werden **nicht** gebraucht.

Die Tests startest du mit `cargo test --release`.

Bei jedem Push baut GitHub Actions beide Versionen. Bei einem Tag `v*` (z. B. `git tag v0.1.0 && git push --tags`) entsteht automatisch ein Release mit beiden Dateien.

---

## 8. Wie es funktioniert

Alles ist auf möglichst wenig Verzögerung ausgelegt:

- **Kein Codec.** Die Stimme geht unkomprimiert über die Leitung (48 kHz, mono, 16 Bit, also 768 kbit/s). Für ein Heimnetz ist das nichts, und es spart die 5 bis 25 ms, die ein Codec wie Opus für Puffern und Vorausschau braucht.
- **UDP direkt von PC zu PC**, ohne Server und ohne TCP-Wartezeiten. Jeder Audio-Callback wird sofort als eigenes Paket verschickt (≤ 2,5 ms Audio pro Paket).
- **miniaudio** als Audio-Schicht:
  - Windows: WASAPI im Low-Latency-Modus (IAudioClient3, MMCSS „Pro Audio“), optional Exclusive.
  - Linux: PulseAudio/PipeWire, JACK oder ALSA mit kleinem Quantum.
- **Adaptiver Jitterpuffer mit Drift-Ausgleich.** Zwei Soundkarten laufen nie exakt gleich schnell; ±100 ppm sind normal und ergäben ohne Ausgleich eine stündlich wachsende Verzögerung. Ein PI-Regler hält deshalb den *minimalen* Füllstand des Puffers auf einer kleinen Reserve. Bei Bedarf lässt er pro Audioblock ein einzelnes Sample weg oder ergänzt eines, per Interpolation über den Block, also unhörbar. Die Reserve wächst automatisch, wenn Aussetzer auftreten, und schrumpft wieder, wenn alles stabil läuft.
- **Echte Latenzmessung.** Der Empfänger meldet 10-mal pro Sekunde, welches Sample (Zeitstempel des Absenders) er gerade abspielt. Daraus ergibt sich ohne Uhrensynchronisation die tatsächliche Verzögerung.

### Projektstruktur
```
native/miniaudio.h   Audio-Bibliothek (Single Header, v0.11.25)
native/ma_shim.c     schlanke C-Schnittstelle für Rust
src/audio.rs         sichere Rust-Hülle um miniaudio
src/protocol.rs      UDP-Paketformat
src/jitter.rs        Jitterpuffer + Drift-Regler (mit Simulationstests)
src/dsp.rs           Verstärkung, Rauschsperre, Push-to-Talk-Logik
src/engine.rs        Netzwerk-Threads, Verbindungsaufbau, Audio-Geräte
src/ptt.rs           globale PTT-Taste (Windows: GetAsyncKeyState, Linux: evdev)
src/ui.rs            Fenster (egui)
src/main.rs          Start, Kommandozeilen-Optionen, Selbsttest
```

### Protokoll
UDP-Port 47800. Jedes Paket hat einen 20-Byte-Header (`IPTK`, Version, Typ, Flags, Sequenznummer, Zeitstempel in µs). Die Typen sind: Audio, Hello/HelloAck (Verbindungsaufbau), Discover/Announce (LAN-Suche per Broadcast), Ping/Pong, Played (Latenzmessung), Stats und Bye.

**Hinweis:** Die Übertragung ist **unverschlüsselt**, gedacht fürs eigene Heimnetz.

## Lizenz
MIT. Siehe [LICENSE](LICENSE). miniaudio steht unter Public Domain bzw. MIT-0.
