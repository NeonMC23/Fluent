#!/usr/bin/env python3
"""Package a staged Fluent.app as a deterministic macOS ZIP."""

from __future__ import annotations

import argparse
import hashlib
import plistlib
import stat
import zipfile
from pathlib import Path

FIXED_TIME = (2020, 1, 1, 0, 0, 0)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--arch", required=True,
                        choices=["arm64", "x86_64"])
    parser.add_argument("--output", default="dist")
    args = parser.parse_args()

    app = args.app
    plist_path = app / "Contents/Info.plist"
    binary = app / "Contents/MacOS/fluent"
    icon = app / "Contents/Resources/Fluent.icns"
    for required in (plist_path, binary, icon):
        if not required.is_file():
            raise SystemExit(f"missing bundle resource: {required}")
    with plist_path.open("rb") as handle:
        plist = plistlib.load(handle)
    if plist.get("CFBundleShortVersionString") != args.version:
        raise SystemExit("Info.plist version does not match --version")
    if plist.get("CFBundleIdentifier") != "io.fluent.Fluent":
        raise SystemExit("unexpected CFBundleIdentifier")
    if plist.get("CFBundleIconFile") != "Fluent.icns":
        raise SystemExit("unexpected CFBundleIconFile")

    outdir = Path(args.output)
    outdir.mkdir(parents=True, exist_ok=True)
    name = f"Fluent-{args.version}-macos-{args.arch}.zip"
    archive = outdir / name
    staging = archive.with_suffix(".zip.tmp")
    with zipfile.ZipFile(staging, "w", zipfile.ZIP_DEFLATED) as zf:
        for path in sorted(app.rglob("*")):
            if path.is_dir():
                continue
            relative = path.relative_to(app)
            member = f"Fluent.app/{relative.as_posix()}"
            data = path.read_bytes()
            info = zipfile.ZipInfo(member, FIXED_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            mode = path.stat().st_mode
            info.external_attr = (mode & 0xFFFF) << 16
            info.create_system = 3
            zf.writestr(info, data)
    staging.replace(archive)
    digest = sha256(archive)
    (outdir / f"{name}.sha256").write_text(f"{digest}  {name}\n", encoding="utf-8")
    print(f"{archive}\nSHA-256 {digest}")


if __name__ == "__main__":
    main()
