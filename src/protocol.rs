//! UDP-Paketformat. Alles Little Endian.
//!
//! ```text
//! 0..4   Magic "IPTK"
//! 4      Version
//! 5      Typ
//! 6      Flags
//! 7      reserviert
//! 8..12  Sequenznummer (u32)
//! 12..20 Zeitstempel in µs (u64, Uhr des Absenders)
//! 20..   Nutzdaten
//! ```

pub const MAGIC: &[u8; 4] = b"IPTK";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 20;
/// Höchstens so viele Samples pro Audiopaket (2,5 ms bei 48 kHz).
pub const MAX_SAMPLES: usize = 120;
pub const MAX_PACKET: usize = HEADER_LEN + MAX_SAMPLES * 2;
pub const MAX_NAME: usize = 48;

/// Erstes Paket nach einer Sprechpause: der Empfänger puffert neu vor.
pub const FLAG_TALKSPURT: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Audio = 1,
    Hello = 2,
    HelloAck = 3,
    Discover = 4,
    Announce = 5,
    Ping = 6,
    Pong = 7,
    Played = 8,
    Stats = 9,
    Bye = 10,
}

impl Kind {
    fn from_u8(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::Audio,
            2 => Kind::Hello,
            3 => Kind::HelloAck,
            4 => Kind::Discover,
            5 => Kind::Announce,
            6 => Kind::Ping,
            7 => Kind::Pong,
            8 => Kind::Played,
            9 => Kind::Stats,
            10 => Kind::Bye,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Header {
    pub kind: Kind,
    pub flags: u8,
    pub seq: u32,
    pub ts: u64,
}

/// Schreibt den Header in `buf` und gibt die Header-Länge zurück.
pub fn write_header(buf: &mut [u8], kind: Kind, flags: u8, seq: u32, ts: u64) -> usize {
    buf[0..4].copy_from_slice(MAGIC);
    buf[4] = VERSION;
    buf[5] = kind as u8;
    buf[6] = flags;
    buf[7] = 0;
    buf[8..12].copy_from_slice(&seq.to_le_bytes());
    buf[12..20].copy_from_slice(&ts.to_le_bytes());
    HEADER_LEN
}

pub fn parse(buf: &[u8]) -> Option<(Header, &[u8])> {
    if buf.len() < HEADER_LEN || &buf[0..4] != MAGIC || buf[4] != VERSION {
        return None;
    }
    let kind = Kind::from_u8(buf[5])?;
    let seq = u32::from_le_bytes(buf[8..12].try_into().ok()?);
    let ts = u64::from_le_bytes(buf[12..20].try_into().ok()?);
    Some((Header { kind, flags: buf[6], seq, ts }, &buf[HEADER_LEN..]))
}

/// Baut ein Audiopaket ohne Allokation (für den Audio-Callback).
pub fn encode_audio(buf: &mut [u8; MAX_PACKET], flags: u8, seq: u32, ts: u64, samples: &[f32]) -> usize {
    debug_assert!(samples.len() <= MAX_SAMPLES);
    let mut n = write_header(buf, Kind::Audio, flags, seq, ts);
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        buf[n..n + 2].copy_from_slice(&v.to_le_bytes());
        n += 2;
    }
    n
}

/// Dekodiert Audio-Nutzdaten in `out` und gibt die Anzahl Samples zurück.
pub fn decode_audio(payload: &[u8], out: &mut [f32]) -> usize {
    let n = (payload.len() / 2).min(out.len());
    for i in 0..n {
        let v = i16::from_le_bytes([payload[2 * i], payload[2 * i + 1]]);
        out[i] = v as f32 / 32768.0;
    }
    n
}

/// Paket mit einem Namen als Nutzdaten (Hello, HelloAck, Announce).
pub fn encode_named(kind: Kind, seq: u32, ts: u64, name: &str) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_LEN];
    write_header(&mut buf, kind, 0, seq, ts);
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    buf.extend_from_slice(&name.as_bytes()[..end]);
    buf
}

pub fn decode_name(payload: &[u8]) -> String {
    String::from_utf8_lossy(&payload[..payload.len().min(MAX_NAME)]).trim().to_string()
}

/// Statistik, die jede Seite der anderen regelmäßig schickt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// Geschätzte Verzögerung des eigenen Kopfhörerpuffers.
    pub playback_device_ms: f32,
    /// Gemessene Latenz in Senderichtung (eigene Stimme bei der Gegenstelle) (NaN, falls unbekannt).
    pub my_to_you_ms: f32,
}

pub fn encode_stats(seq: u32, ts: u64, s: &Stats) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_LEN];
    write_header(&mut buf, Kind::Stats, 0, seq, ts);
    buf.extend_from_slice(&s.playback_device_ms.to_le_bytes());
    buf.extend_from_slice(&s.my_to_you_ms.to_le_bytes());
    buf
}

pub fn decode_stats(payload: &[u8]) -> Option<Stats> {
    if payload.len() < 8 {
        return None;
    }
    Some(Stats {
        playback_device_ms: f32::from_le_bytes(payload[0..4].try_into().ok()?),
        my_to_you_ms: f32::from_le_bytes(payload[4..8].try_into().ok()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_roundtrip() {
        let samples: Vec<f32> = (0..MAX_SAMPLES).map(|i| (i as f32 / 60.0) - 1.0).collect();
        let mut buf = [0u8; MAX_PACKET];
        let n = encode_audio(&mut buf, FLAG_TALKSPURT, 42, 123_456, &samples);
        assert_eq!(n, MAX_PACKET);
        let (h, payload) = parse(&buf[..n]).unwrap();
        assert_eq!(h, Header { kind: Kind::Audio, flags: FLAG_TALKSPURT, seq: 42, ts: 123_456 });
        let mut out = [0f32; MAX_SAMPLES];
        assert_eq!(decode_audio(payload, &mut out), MAX_SAMPLES);
        for (a, b) in samples.iter().zip(out.iter()) {
            assert!((a - b).abs() < 1.0 / 16000.0);
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"hello world, this is not iptalk").is_none());
        assert!(parse(b"IPTK").is_none());
        let mut buf = encode_named(Kind::Hello, 1, 2, "x");
        buf[4] = 99;
        assert!(parse(&buf).is_none());
    }

    #[test]
    fn named_and_stats() {
        let buf = encode_named(Kind::Announce, 7, 8, "Raphaels PC äöü");
        let (h, p) = parse(&buf).unwrap();
        assert_eq!(h.kind, Kind::Announce);
        assert_eq!(decode_name(p), "Raphaels PC äöü");

        let s = Stats { playback_device_ms: 2.5, my_to_you_ms: 9.75 };
        let buf = encode_stats(1, 2, &s);
        let (_, p) = parse(&buf).unwrap();
        assert_eq!(decode_stats(p), Some(s));
    }

    #[test]
    fn long_name_cut_on_char_boundary() {
        let name = "ä".repeat(40); // 80 Bytes
        let buf = encode_named(Kind::Hello, 0, 0, &name);
        let (_, p) = parse(&buf).unwrap();
        assert_eq!(decode_name(p), "ä".repeat(24));
    }
}
