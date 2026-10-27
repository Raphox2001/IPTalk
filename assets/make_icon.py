"""Zeichnet das IPTalk-Icon (Headset mit Schallwellen) ohne externe Pakete.

Ausgabe in diesem Ordner:
  iptalk.ico        Windows-Icon (16 bis 256 px, PNG-komprimiert)
  iptalk.png        256 px, für Linux-Startmenü und README
  iptalk-64.rgba    64 px Rohdaten (RGBA) für das Fenster-Icon

Aufruf: python assets/make_icon.py
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))

# Farbverlauf des Hintergrunds (oben links → unten rechts)
C0 = (37, 99, 235)    # Blau
C1 = (13, 148, 136)   # Türkis


def clamp(v, lo=0.0, hi=1.0):
    return lo if v < lo else hi if v > hi else v


def sd_round_rect(px, py, cx, cy, hw, hh, r):
    qx = abs(px - cx) - (hw - r)
    qy = abs(py - cy) - (hh - r)
    return math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - r


def sd_capsule(px, py, x, y0, y1, r):
    """Senkrechte Kapsel von y0 bis y1."""
    cy = clamp(py, y0, y1)
    return math.hypot(px - x, py - cy) - r


def sd_arc(px, py, cx, cy, r, w):
    """Obere Halbkreis-Linie (Kopfbügel) mit runden Enden."""
    if py <= cy:
        return abs(math.hypot(px - cx, py - cy) - r) - w / 2
    d1 = math.hypot(px - (cx - r), py - cy)
    d2 = math.hypot(px - (cx + r), py - cy)
    return min(d1, d2) - w / 2


def coverage(d, px_size):
    # Kantenglättung: Abstand in Pixeln → Deckkraft
    return clamp(0.5 - d / px_size)


def render(size):
    px = 1.0 / size
    rows = []
    bars = [(0.395, 0.055), (0.4475, 0.105), (0.5, 0.15), (0.5525, 0.105), (0.605, 0.055)]
    for j in range(size):
        row = bytearray()
        y = (j + 0.5) / size
        for i in range(size):
            x = (i + 0.5) / size
            # Hintergrund: abgerundetes Quadrat mit Verlauf
            bg = coverage(sd_round_rect(x, y, 0.5, 0.5, 0.46, 0.46, 0.2), px)
            t = clamp((x + y) / 2)
            r = C0[0] + (C1[0] - C0[0]) * t
            g = C0[1] + (C1[1] - C0[1]) * t
            b = C0[2] + (C1[2] - C0[2]) * t
            # leichter Glanz oben
            shine = 0.10 * clamp(1.0 - y / 0.55)
            r, g, b = r + (255 - r) * shine, g + (255 - g) * shine, b + (255 - b) * shine

            # Headset: Bügel + Ohrmuscheln
            d = sd_arc(x, y, 0.5, 0.55, 0.27, 0.06)
            d = min(d, sd_round_rect(x, y, 0.23, 0.63, 0.065, 0.12, 0.055))
            d = min(d, sd_round_rect(x, y, 0.77, 0.63, 0.065, 0.12, 0.055))
            # Schallwellen zwischen den Ohrmuscheln
            for bx, h in bars:
                d = min(d, sd_capsule(x, y, bx, 0.63 - h / 2, 0.63 + h / 2, 0.018))
            fg = coverage(d, px)

            r = r + (255 - r) * fg
            g = g + (255 - g) * fg
            b = b + (255 - b) * fg
            row += bytes((int(r + 0.5), int(g + 0.5), int(b + 0.5), int(bg * 255 + 0.5)))
        rows.append(bytes(row))
    return rows


def png_bytes(rows, size):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def main():
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = {s: render(s) for s in sizes}
    pngs = {s: png_bytes(images[s], s) for s in sizes}

    # ICO: Kopf, Verzeichnis, dann die PNG-Daten
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries, data = b"", b""
    for s in sizes:
        p = pngs[s]
        dim = 0 if s == 256 else s
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(p), offset)
        offset += len(p)
        data += p
    with open(os.path.join(HERE, "iptalk.ico"), "wb") as f:
        f.write(header + entries + data)
    with open(os.path.join(HERE, "iptalk.png"), "wb") as f:
        f.write(pngs[256])
    with open(os.path.join(HERE, "iptalk-64.rgba"), "wb") as f:
        f.write(b"".join(images[64]))
    print("Icons geschrieben nach", HERE)


if __name__ == "__main__":
    main()
