"""Gera assets/TrayTemp.ico (termômetro) sem dependências externas.

Uso: python tools/make_icon.py
"""
import math
import struct
import sys
import zlib
from pathlib import Path

SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
SUPERSAMPLE = 4

TOP = (0xFF, 0x8A, 0x3D)
BOTTOM = (0xE0, 0x30, 0x2A)
GLYPH = (0xFF, 0xFF, 0xFF)
MERCURY = (0xD8, 0x26, 0x22)


def sd_round_rect(x, y, half, radius):
    qx, qy = abs(x - 0.5) - half + radius, abs(y - 0.5) - half + radius
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - radius


def sd_capsule(x, y, top, bottom, cx, radius):
    py = min(max(y, top), bottom)
    return math.hypot(x - cx, y - py) - radius


def sd_circle(x, y, cx, cy, radius):
    return math.hypot(x - cx, y - cy) - radius


def shade(x, y):
    """Cor RGBA (0..1) do ponto (x, y) no quadrado unitário."""
    if sd_round_rect(x, y, 0.5, 0.22) > 0:
        return (0, 0, 0, 0)
    t = y
    color = tuple(TOP[i] + (BOTTOM[i] - TOP[i]) * t for i in range(3))
    # Corpo do termômetro: tubo + bulbo
    body = min(sd_capsule(x, y, 0.17, 0.62, 0.5, 0.115), sd_circle(x, y, 0.5, 0.71, 0.175))
    if body < 0:
        color = GLYPH
        # Mercúrio: subindo pela metade do tubo até o bulbo
        mercury = min(sd_capsule(x, y, 0.38, 0.62, 0.5, 0.05), sd_circle(x, y, 0.5, 0.71, 0.105))
        if mercury < 0:
            color = MERCURY
    return (*(c / 255 for c in color), 1.0)


def render(size):
    n = SUPERSAMPLE
    pixels = bytearray()
    for py in range(size):
        row = bytearray([0])  # filtro PNG "None"
        for px in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(n):
                for sx in range(n):
                    r, g, b, a = shade((px + (sx + 0.5) / n) / size, (py + (sy + 0.5) / n) / size)
                    acc[0] += r * a
                    acc[1] += g * a
                    acc[2] += b * a
                    acc[3] += a
            a = acc[3] / (n * n)
            if a > 0:
                rgb = [round(255 * acc[i] / acc[3]) for i in range(3)]
            else:
                rgb = [0, 0, 0]
            row += bytes([*rgb, round(255 * a)])
        pixels += row
    return pixels


def png(size, raw):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")


def main():
    images = [png(s, render(s)) for s in SIZES]
    out = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    offset = 6 + 16 * len(images)
    for size, data in zip(SIZES, images):
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    for data in images:
        out += data
    target = Path(__file__).resolve().parent.parent / "assets" / "TrayTemp.ico"
    target.write_bytes(out)
    print(f"{target} ({len(out)} bytes)")
    # Prévia opcional: python tools/make_icon.py <pasta> grava cada tamanho como PNG
    if len(sys.argv) > 1:
        for size, data in zip(SIZES, images):
            (Path(sys.argv[1]) / f"icon_{size}.png").write_bytes(data)


if __name__ == "__main__":
    main()
