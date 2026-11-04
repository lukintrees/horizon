#!/usr/bin/env python3
"""Independent CPU renderer for the PR 6 legacy-GL golden frame.

This script deliberately does not import the Rust `gfx21` module. It draws the
same hard-coded reference scene with integer arithmetic and writes:

  benchmarks/graphics/legacy-gl-scene.rgba  (raw RGBA8, top-down)
  benchmarks/graphics/legacy-gl-scene.png   (RGBA8 PNG, filter 0)

CI writes the Rust rasterizer's bytes to a temporary file and calls `--check`.
The frame is a deterministic command-stream/CPU reference, not a wgpu parity
image; real X1650 screenshots remain a separate acceptance step.
"""

import argparse
import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RGBA_PATH = ROOT / "benchmarks/graphics/legacy-gl-scene.rgba"
PNG_PATH = ROOT / "benchmarks/graphics/legacy-gl-scene.png"
WIDTH = 80
HEIGHT = 60

SKY_TOP = (24, 36, 56)
SKY_BOTTOM = (72, 96, 120)
FOG_COLOR = (90, 110, 130)

OPAQUE = "opaque"
CUTOUT = "cutout"
ALPHA = "alpha"

# x0, y0, x1, y1, RGBA, fog (distance*255/100), blend, checker skip.
QUADS = (
    (8, 20, 32, 44, (120, 90, 60, 255), 51, OPAQUE, False),
    (20, 16, 44, 40, (180, 160, 120, 255), 76, CUTOUT, True),
    (40, 24, 64, 48, (40, 90, 160, 128), 102, ALPHA, False),
)
HUD = ((4, 4, 20, 12, (220, 220, 220, 255)),)
LINES = ((4, 56, 76, 57, (240, 200, 80, 255)),)


def sky_pixel(y):
    t = y * 255 // max(HEIGHT - 1, 1)
    return tuple(SKY_TOP[i] + (SKY_BOTTOM[i] - SKY_TOP[i]) * t // 255 for i in range(3))


def apply_fog(color, fog):
    inv = 255 - fog
    return tuple((color[i] * inv + FOG_COLOR[i] * fog) // 255 for i in range(3)) + (color[3],)


def blend_channel(dst, src, alpha):
    return (src * alpha + dst * (255 - alpha)) // 255


def fill_rect(out, rect):
    x0, y0, x1, y1, color = rect
    x0 = min(WIDTH, max(0, x0))
    y0 = min(HEIGHT, max(0, y0))
    x1 = min(WIDTH, max(0, x1))
    y1 = min(HEIGHT, max(0, y1))
    if x0 >= x1 or y0 >= y1:
        return
    pixel_bytes = bytes(color)
    for y in range(y0, y1):
        for x in range(x0, x1):
            at = (y * WIDTH + x) * 4
            out[at:at + 4] = pixel_bytes


def render():
    out = bytearray(WIDTH * HEIGHT * 4)
    for y in range(HEIGHT):
        row = bytes(sky_pixel(y) + (255,))
        for x in range(WIDTH):
            at = (y * WIDTH + x) * 4
            out[at:at + 4] = row

    for x0, y0, x1, y1, color, fog, blend, checker in QUADS:
        color = apply_fog(color, fog)
        x0 = min(WIDTH, max(0, x0))
        y0 = min(HEIGHT, max(0, y0))
        x1 = min(WIDTH, max(0, x1))
        y1 = min(HEIGHT, max(0, y1))
        if x0 >= x1 or y0 >= y1:
            continue
        opaque_bytes = bytes(color)
        for y in range(y0, y1):
            for x in range(x0, x1):
                if checker and (x + y) % 2 == 0:
                    continue
                at = (y * WIDTH + x) * 4
                if blend in (OPAQUE, CUTOUT):
                    out[at:at + 4] = opaque_bytes
                else:
                    out[at] = blend_channel(out[at], color[0], color[3])
                    out[at + 1] = blend_channel(out[at + 1], color[1], color[3])
                    out[at + 2] = blend_channel(out[at + 2], color[2], color[3])

    for rect in HUD:
        fill_rect(out, rect)
    for rect in LINES:
        fill_rect(out, rect)
    return bytes(out)


def chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)


def write_png(path, width, height, rgba):
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)  # filter type 0
        raw.extend(rgba[y * stride:(y + 1) * stride])
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
           + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))
    path.write_bytes(png)


def read_png_rgba(path):
    data = path.read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("not a PNG file")
    pos = 8
    width = height = None
    idat = bytearray()
    while pos + 12 <= len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        tag = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if tag == b"IHDR":
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", payload)
            if (depth, color, compression, filtering, interlace) != (8, 6, 0, 0, 0):
                raise ValueError("expected a non-interlaced RGBA8 PNG with filter 0")
        elif tag == b"IDAT":
            idat.extend(payload)
        elif tag == b"IEND":
            break
    if width is None or height is None:
        raise ValueError("PNG has no IHDR")
    raw = zlib.decompress(bytes(idat))
    stride = width * 4
    if len(raw) != height * (stride + 1):
        raise ValueError("unexpected PNG scanline length")
    out = bytearray()
    for y in range(height):
        start = y * (stride + 1)
        if raw[start] != 0:
            raise ValueError("only filter 0 is supported")
        out.extend(raw[start + 1:start + 1 + stride])
    return width, height, bytes(out)


def generate():
    expected = render()
    RGBA_PATH.parent.mkdir(parents=True, exist_ok=True)
    RGBA_PATH.write_bytes(expected)
    write_png(PNG_PATH, WIDTH, HEIGHT, expected)
    width, height, decoded = read_png_rgba(PNG_PATH)
    if (width, height, decoded) != (WIDTH, HEIGHT, expected):
        raise ValueError("generated PNG does not round-trip")
    print(f"golden frame generated: {len(expected)} RGBA bytes, {width}x{height} PNG")
    return 0


def check(raw_path):
    expected = render()
    actual = Path(raw_path).read_bytes()
    if actual != expected:
        for i, (a, b) in enumerate(zip(actual, expected)):
            if a != b:
                raise ValueError(f"RGBA mismatch at byte {i}: got {a}, expected {b}")
        raise ValueError(f"RGBA length mismatch: got {len(actual)}, expected {len(expected)}")
    width, height, decoded = read_png_rgba(PNG_PATH)
    if (width, height, decoded) != (WIDTH, HEIGHT, expected):
        raise ValueError("committed PNG does not match the independent frame")
    print(f"golden frame: OK ({len(expected)} RGBA bytes, PNG {width}x{height})")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate", action="store_true", help="write the RGBA and PNG fixtures")
    mode.add_argument("--check", nargs="?", const=str(RGBA_PATH), metavar="RGBA",
                      help="compare a raw RGBA file and the committed PNG with the independent frame")
    args = parser.parse_args()
    try:
        return generate() if args.generate else check(args.check)
    except (OSError, ValueError) as err:
        print(f"golden frame: FAILED: {err}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
