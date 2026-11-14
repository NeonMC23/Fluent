#!/usr/bin/env python3
"""Package a native Windows release executable as a portable ZIP.

The binary must be a real PE executable; the script is run on the Windows CI
runner after `cargo build --release`.
"""

from __future__ import annotations

import argparse
import hashlib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXED_TIME = (2020, 1, 1, 0, 0, 0)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--arch", default="x86_64")
    parser.add_argument("--output", default=str(ROOT / "dist"))
    args = parser.parse_args()

    binary = args.binary
    if not binary.is_file():
        raise SystemExit(f"binary not found: {binary}")
    if binary.read_bytes()[:2] != b"MZ":
        raise SystemExit(f"not a PE executable: {binary}")

    outdir = Path(args.output)
    outdir.mkdir(parents=True, exist_ok=True)
    name = f"fluent-{args.version}-windows-{args.arch}.zip"
    archive = outdir / name
    readme = f"""Fluent {args.version} - Windows {args.arch} portable build
==============================================================

This ZIP contains the application only. It does NOT contain NLLB-200 models,
tokenizer files, configuration, user data or an installer.

Run
---
1. Extract the ZIP to a folder of your choice.
2. Place model directories next to fluent.exe:

       models/nllb-200-1.3b/
           config.json
           tokenizer.json
           pytorch_model.bin   (or safetensors / sharded checkpoint)
       models/nllb-200-3.3b/
           ...

   Fluent does not download models. Their license is CC-BY-NC-4.0; see the
   project documentation. Only config.json, tokenizer.json and a complete
   checkpoint are required.

3. Run:

       fluent.exe

The GUI starts without a model and reports an actionable missing-model error
on the first translation.

Verification
------------
Use the adjacent .sha256 file:

    Get-FileHash .\\{name} -Algorithm SHA256

A checksum verifies integrity relative to the channel that supplied it; it is
not a signature. If a .minisig file is present, verify it with Minisign and the
project public key.

License
-------
The Fluent source is MIT-licensed. NLLB-200 model/tokenizer assets are not
included and remain subject to Meta's terms.
"""
    staging = archive.with_suffix(".zip.tmp")
    with zipfile.ZipFile(staging, "w", zipfile.ZIP_DEFLATED) as zf:
        for member, data in (
            ("fluent.exe", binary.read_bytes()),
            ("LICENSE", (ROOT / "LICENSE").read_bytes()),
            ("README.txt", readme.encode("utf-8")),
        ):
            info = zipfile.ZipInfo(member, FIXED_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o644 & 0xFFFF) << 16
            if member == "fluent.exe":
                info.external_attr = (0o755 & 0xFFFF) << 16
            zf.writestr(info, data)
    staging.replace(archive)
    digest = sha256(archive)
    (outdir / f"{name}.sha256").write_text(f"{digest}  {name}\n", encoding="utf-8")
    print(f"{archive}\nSHA-256 {digest}")


if __name__ == "__main__":
    main()
