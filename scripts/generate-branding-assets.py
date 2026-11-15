#!/usr/bin/env python3
"""Generate Fluent platform icon assets from the immutable PNG masters.

Requires Pillow (development-only). Generated files are committed so normal
builds and installers do not need Pillow.

The masters are never modified:
    src/logo/logo.png    1024x1024 RGBA (mark + wordmark)
    src/logo/banner.png  1536x1024 RGBA (landscape banner)

The canonical application symbol is the turquoise F mark. It is extracted
byte-for-byte from the master with the same crop the GUI has always used and
stored as `src/logo/fluent-mark.png`. Every platform icon derives from that
mark, not from the full wordmark artwork.

Derived output:
    src/logo/fluent-mark.png
    src/logo/generated/png/fluent-<size>.png
    src/logo/generated/window/fluent-256.rgba
    src/logo/generated/ui/fluent-mark-256.rgba
    src/logo/generated/ui/fluent-mark-256.png
    src/logo/generated/windows/fluent.ico
    src/logo/generated/macos/Fluent.icns
"""

from __future__ import annotations

import hashlib
import struct
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / "src/logo/logo.png"
MARK_REL = Path("src/logo/fluent-mark.png")
GENERATED_REL = Path("src/logo/generated")

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


def outputs() -> list[Path]:
    paths = [MARK_REL]
    paths += [GENERATED_REL / "png" / f"fluent-{size}.png" for size in PNG_SIZES]
    paths += [
        GENERATED_REL / "window/fluent-256.rgba",
        GENERATED_REL / "ui/fluent-mark-256.png",
        GENERATED_REL / "ui/fluent-mark-256.rgba",
        GENERATED_REL / "windows/fluent.ico",
        GENERATED_REL / "macos/Fluent.icns",
    ]
    return paths


def generate(out_root: Path) -> None:
    if not MASTER.is_file():
        raise SystemExit(f"missing master: {MASTER}")
    master = Image.open(MASTER).convert("RGBA")
    if master.size != (1024, 1024):
        raise SystemExit(f"unexpected logo dimensions: {master.size}")

    mark_path = out_root / MARK_REL
    png_dir = out_root / GENERATED_REL / "png"
    window_dir = out_root / GENERATED_REL / "window"
    ui_dir = out_root / GENERATED_REL / "ui"
    ico = out_root / GENERATED_REL / "windows/fluent.ico"
    icns = out_root / GENERATED_REL / "macos/Fluent.icns"

    for directory in (mark_path.parent, png_dir, window_dir, ui_dir, ico.parent, icns.parent):
        directory.mkdir(parents=True, exist_ok=True)

    # The immutable master contains the mark and the wordmark. The selected
    # application symbol is the mark; extract the exact crop the GUI header
    # has used since Cycle 1. The master itself is never written.
    mark = master.crop((270, 190, 745, 665))
    mark.save(mark_path, format="PNG", optimize=True)

    for size in PNG_SIZES:
        (png_dir / f"fluent-{size}.png").write_bytes(make_png_bytes(mark, size))

    # Raw RGBA for compile-time embedding in the GUI (no image decoder needed).
    (window_dir / "fluent-256.rgba").write_bytes(
        mark.resize((256, 256), Image.Resampling.LANCZOS).tobytes()
    )

    mark_256 = mark.resize((256, 256), Image.Resampling.LANCZOS)
    mark_256.save(ui_dir / "fluent-mark-256.png", format="PNG", optimize=True)
    (ui_dir / "fluent-mark-256.rgba").write_bytes(mark_256.tobytes())

    mark.save(ico, format="ICO", sizes=ICO_SIZES, bitmap_format="bmp")

    chunks = []
    for kind, size in ICNS_ENTRIES:
        payload = make_png_bytes(mark, size)
        chunks.append(kind.encode("ascii") + struct.pack(">I", 8 + len(payload)) + payload)
    body = b"".join(chunks)
    icns.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)

    print(f"master sha256: {sha256(MASTER)}")
    print(f"mark sha256:   {sha256(mark_path)} ({mark.size[0]}x{mark.size[1]})")
    print(f"generated: {len(PNG_SIZES)} PNG, 1 mark PNG/RGBA, 1 window RGBA, 1 ICO, 1 ICNS")
    print(f"ICO sha256: {sha256(ico)}")
    print(f"ICNS sha256: {sha256(icns)}")


def check() -> None:
    with tempfile.TemporaryDirectory(prefix="fluent-branding-") as temp:
        temp_root = Path(temp)
        generate(temp_root)
        failures = []
        for relative in outputs():
            expected = temp_root / relative
            actual = ROOT / relative
            if not actual.is_file():
                failures.append(f"missing {relative}")
            elif expected.read_bytes() != actual.read_bytes():
                failures.append(f"stale {relative}")
        if failures:
            for failure in failures:
                print(f"FAIL: {failure}", file=sys.stderr)
            raise SystemExit(1)
        print(f"PASS: {len(outputs())} branding outputs are reproducible from the master")


if __name__ == "__main__":
    try:
        if "--check" in sys.argv[1:]:
            check()
        else:
            generate(ROOT)
    except ModuleNotFoundError:
        print("Pillow is required: python3 -m pip install pillow", file=sys.stderr)
        raise
