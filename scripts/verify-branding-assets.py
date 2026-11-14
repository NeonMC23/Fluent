#!/usr/bin/env python3
"""Validate the committed Fluent branding assets without Pillow.

Checks source master hashes/dimensions, PNG variants, the ICO directory and
BMP entries, the ICNS chunk table and embedded PNG dimensions, and the raw
RGBA window icon. Standard library only.
"""

from __future__ import annotations

import hashlib
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOGO = ROOT / "src/logo/logo.png"
BANNER = ROOT / "src/logo/banner.png"
GENERATED = ROOT / "src/logo/generated"
PNG_DIR = GENERATED / "png"
ICO = GENERATED / "windows/fluent.ico"
ICNS = GENERATED / "macos/Fluent.icns"
RGBA = GENERATED / "window/fluent-256.rgba"

MASTER_SHA = "bce82ebd549e4e6a0001a605a7bb8ff3efc8bfedf30fc20c3803801db884b5e7"
BANNER_SHA = "ce3deeb9bab7a9cf847ba06568886e8cb5b10d88080bc1c55363300f35925545"
PNG_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
ICNS_ENTRIES = {
    "icp4": 16,
    "icp5": 32,
    "icp6": 64,
    "ic07": 128,
    "ic08": 256,
    "ic09": 512,
    "ic10": 1024,
    "ic11": 32,
    "ic12": 64,
    "ic13": 256,
    "ic14": 512,
}


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def png_dims(data: bytes, label: str) -> tuple[int, int]:
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        fail(f"{label}: not a PNG")
    return struct.unpack(">II", data[16:24])


def check_master(path: Path, expected_sha: str, expected: tuple[int, int]) -> None:
    if not path.is_file():
        fail(f"missing master: {path}")
    if sha256(path) != expected_sha:
        fail(f"master hash changed: {path}")
    if png_dims(path.read_bytes(), str(path)) != expected:
        fail(f"master dimensions changed: {path}")


def check_pngs() -> None:
    for size in PNG_SIZES:
        path = PNG_DIR / f"fluent-{size}.png"
        if not path.is_file():
            fail(f"missing variant: {path}")
        if png_dims(path.read_bytes(), str(path)) != (size, size):
            fail(f"wrong dimensions: {path}")


def check_ico() -> None:
    raw = ICO.read_bytes()
    if len(raw) < 6:
        fail("ICO too short")
    reserved, kind, count = struct.unpack("<HHH", raw[:6])
    if reserved != 0 or kind != 1:
        fail("ICO header is not an icon")
    expected_count = len(ICO_SIZES)
    if count != expected_count:
        fail(f"ICO has {count} entries, expected {expected_count}")
    found = []
    for index in range(count):
        entry = raw[6 + index * 16 : 22 + index * 16]
        width, height, colors, reserved_byte, planes, bits, length, offset = struct.unpack(
            "<BBBBHHII", entry
        )
        width = width or 256
        height = height or 256
        if reserved_byte != 0 or planes not in (0, 1) or bits != 32:
            fail(f"ICO entry {index} has unexpected header: {planes=} {bits=}")
        payload = raw[offset : offset + length]
        if payload[:8] == b"\x89PNG\r\n\x1a\n":
            actual = png_dims(payload, f"ICO entry {index}")
        else:
            # BITMAPINFOHEADER: size, width, height (XOR+AND), planes, bitcount
            if len(payload) < 40:
                fail(f"ICO entry {index} is truncated")
            header_size, bmp_w, bmp_h, bmp_planes, bmp_bits = struct.unpack(
                "<IiiHH", payload[:16]
            )
            if header_size < 40:
                fail(f"ICO entry {index} is not a BITMAPINFOHEADER")
            actual = (bmp_w, abs(bmp_h) // 2)
            if bmp_planes not in (0, 1) or bmp_bits != 32:
                fail(f"ICO entry {index} is not 32-bit")
        if actual != (width, height):
            fail(f"ICO entry {index} payload is {actual}, directory says {(width, height)}")
        found.append(width)
    if sorted(found) != ICO_SIZES:
        fail(f"ICO sizes are {sorted(found)}, expected {ICO_SIZES}")


def check_icns() -> None:
    raw = ICNS.read_bytes()
    if raw[:4] != b"icns":
        fail("ICNS magic is missing")
    total = struct.unpack(">I", raw[4:8])[0]
    if total != len(raw):
        fail(f"ICNS declared length {total}, file is {len(raw)}")
    offset = 8
    seen: dict[str, int] = {}
    while offset < len(raw):
        if offset + 8 > len(raw):
            fail("truncated ICNS chunk header")
        kind = raw[offset : offset + 4].decode("ascii", "replace")
        length = struct.unpack(">I", raw[offset + 4 : offset + 8])[0]
        if length < 8 or offset + length > len(raw):
            fail(f"invalid ICNS chunk length for {kind}")
        payload = raw[offset + 8 : offset + length]
        if kind in ICNS_ENTRIES:
            dims = png_dims(payload, f"ICNS {kind}")
            if dims[0] != dims[1] or dims[0] != ICNS_ENTRIES[kind]:
                fail(f"ICNS {kind} has payload {dims}")
            seen[kind] = dims[0]
        offset += length
    if offset != len(raw):
        fail("ICNS chunks do not end at EOF")
    missing = set(ICNS_ENTRIES) - set(seen)
    if missing:
        fail(f"ICNS is missing entries: {sorted(missing)}")


def check_rgba() -> None:
    if RGBA.stat().st_size != 256 * 256 * 4:
        fail("raw RGBA window icon has the wrong size")


def main() -> None:
    check_master(LOGO, MASTER_SHA, (1024, 1024))
    check_master(BANNER, BANNER_SHA, (1536, 1024))
    check_pngs()
    check_ico()
    check_icns()
    check_rgba()
    print(
        "PASS: branding masters, 8 PNG variants, 7-entry ICO, 11-chunk ICNS "
        "and 256x256 raw RGBA are valid"
    )


if __name__ == "__main__":
    main()
