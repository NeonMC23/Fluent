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

This ZIP contains the application only. It does NOT contain model weights,
tokenizer files, configuration, user data or an installer.

Run
---
1. Extract the ZIP to a folder of your choice.
2. Run:

       fluent.exe

3. Install models with the built-in Model Manager:

       Models (header) or Settings -> Manage models...

   Choose a translation or chat model, review its licence and download size,
   and confirm. Downloads are resumable and verified with SHA-256 before the
   model is reported as installed. Models are stored outside the application
   folder (default under your per-user application data directory).

   Translation models:
     NLLB-200 1.3B / 3.3B   CC-BY-NC-4.0
     MADLAD-400 3B          Apache-2.0
     SMaLL-100              MIT

   Chat models:
     Qwen2.5 1.5B Instruct  Apache-2.0
     Qwen2.5 0.5B Instruct  Apache-2.0

   To use an existing model directory instead, create config.json next to
   fluent.exe or set FLUENT_CONFIG to its absolute path. Example entries:

       "models": {
         "fast": "C:/models/nllb-200-1.3b",
         "accurate": "C:/models/nllb-200-3.3b",
         "madlad": "C:/models/madlad400-3b-mt",
         "small": "C:/models/small100",
         "chat": "C:/models/qwen2.5-1.5b-instruct"
       }

   Fluent starts without models. The Model Manager and first-run guidance show
   what to install; a translation or chat request without the required model
   reports an actionable error.

Verification
------------
Use the adjacent .sha256 file:

    Get-FileHash .\\{name} -Algorithm SHA256

A checksum verifies integrity relative to the channel that supplied it; it is
not a signature. If a .minisig file is present, verify it with Minisign and the
project public key.

License
-------
The Fluent source is MIT-licensed. Model weights are not included. NLLB-200 is
CC-BY-NC-4.0; MADLAD-400 is Apache-2.0; SMaLL-100 is MIT; Qwen2.5 Instruct is
Apache-2.0. Fluent's MIT licence does not relicense model assets.
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
