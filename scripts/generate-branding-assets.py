#!/usr/bin/env python3
"""Generate Fluent platform icon assets from the immutable PNG masters.

Requires Pillow (development-only). Generated files are committed so normal
builds and installers do not need Pillow.

The masters are never modified:
    src/logo/logo.png    1024x1024 RGBA
    src/logo/banner.png  1536x1024 RGBA

Derived output:
    src/logo/generated/png/fluent-<size>.png
    src/logo/generated/window/fluent-256.rgba
    src/logo/generated/ui/fluent-mark-256.rgba
    src/logo/generated/ui/fluent-mark-256.png
    src/logo/generated/windows/fluent.ico
    src/logo/generated/macos/Fluent.icns

The UI mark is a faithful crop of the mark above the wordmark (the wordmark is
already present in the UI header), not a redraw.
"""

from __future__ import annotations

import hashlib
import struct
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / "src/logo/logo.png"
GENERATED = ROOT / "src/logo/generated"
PNG_DIR = GENERATED / "png"
WINDOW_DIR = GENERATED / "window"
UI_DIR = GENERATED / "ui"
ICO = GENERATED / "windows/fluent.ico"
ICNS = GENERATED / "macos/Fluent.icns"

PNG_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
ICO_SIZES = ((16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256))
ICNS_ENTRIES = (
    ("icp4", 16),
    ("icp5", 32),
    ("icp6", 64),
    ("ic07", 128),
    ("ic08", 256),
    ("ic09", 512),
    ("ic10", 1024),
    ("ic11", 32),
    ("ic12", 64),
    ("ic13", 256),
    ("ic14", 512),
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def png_size(path: Path) -> tuple[int, int]:
    raw = path.read_bytes()
    if raw[:8] != b"\x89PNG\r\n\x1a\n" or raw[12:16] != b"IHDR":
        raise SystemExit(f"not a PNG: {path}")
    return struct.unpack(">II", raw[16:24])


def make_png_bytes(image: Image.Image, size: int) -> bytes:
    import io

    buf = io.BytesIO()
    image.resize((size, size), Image.Resampling.LANCZOS).save(
        buf, format="PNG", optimize=True
    )
    return buf.getvalue()


def generate() -> None:
    if not MASTER.is_file():
        raise SystemExit(f"missing master: {MASTER}")
    master = Image.open(MASTER).convert("RGBA")
    if master.size != (1024, 1024):
        raise SystemExit(f"unexpected logo dimensions: {master.size}")

    for directory in (PNG_DIR, WINDOW_DIR, UI_DIR, ICO.parent, ICNS.parent):
        directory.mkdir(parents=True, exist_ok=True)

    for size in PNG_SIZES:
        resized = master.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(PNG_DIR / f"fluent-{size}.png", format="PNG", optimize=True)

    # Raw RGBA for compile-time embedding in the GUI (no image decoder needed).
    (WINDOW_DIR / "fluent-256.rgba").write_bytes(
        master.resize((256, 256), Image.Resampling.LANCZOS).tobytes()
    )

    # The header uses the mark only; the wordmark is rendered as text next to
    # it. Crop the mark above the wordmark, preserving the official artwork.
    mark = master.crop((270, 190, 745, 665))
    mark_256 = mark.resize((256, 256), Image.Resampling.LANCZOS)
    mark_256.save(UI_DIR / "fluent-mark-256.png", format="PNG", optimize=True)
    (UI_DIR / "fluent-mark-256.rgba").write_bytes(mark_256.tobytes())

    master.save(ICO, format="ICO", sizes=ICO_SIZES, bitmap_format="bmp")

    chunks = []
    for kind, size in ICNS_ENTRIES:
        payload = make_png_bytes(master, size)
        chunks.append(kind.encode("ascii") + struct.pack(">I", 8 + len(payload)) + payload)
    body = b"".join(chunks)
    ICNS.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)

    print(f"master sha256: {sha256(MASTER)}")
    print(f"generated: {len(PNG_SIZES)} PNG, 1 mark PNG/RGBA, 1 window RGBA, 1 ICO, 1 ICNS")
    print(f"ICO sha256: {sha256(ICO)}")
    print(f"ICNS sha256: {sha256(ICNS)}")


if __name__ == "__main__":
    try:
        generate()
    except ModuleNotFoundError:
        print("Pillow is required: python3 -m pip install pillow", file=sys.stderr)
        raise
