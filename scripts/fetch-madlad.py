#!/usr/bin/env python3
"""Download the pinned MADLAD-400-3B-MT q4_k GGUF model and tokenizer.

Weights stay outside the Git repository. Downloads resume from a `.part`
file, are checked against the pinned Hugging Face revision, size and SHA-256,
and are only renamed into place after verification.

Usage:
    python3 scripts/fetch-madlad.py
    python3 scripts/fetch-madlad.py --dest ~/.local/share/fluent/models/madlad400-3b-mt
    python3 scripts/fetch-madlad.py --check
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "madlad-files.json"
BASE_URL = "https://huggingface.co/{repo}/resolve/{revision}/{name}"
MARGIN = 512 * 1024 * 1024


def default_dest() -> Path:
    data_home = os.environ.get("XDG_DATA_HOME")
    root = Path(data_home) if data_home else Path.home() / ".local" / "share"
    return root / "fluent" / "models" / "madlad400-3b-mt"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_manifest() -> dict:
    return json.loads(MANIFEST.read_text(encoding="utf-8"))


def is_valid(path: Path, entry: dict) -> bool:
    return (
        path.is_file()
        and path.stat().st_size == entry["size"]
        and sha256(path) == entry["sha256"]
    )


def download(url: str, target: Path, entry: dict) -> None:
    part = target.with_name(target.name + ".part")
    command = [
        "curl",
        "--location",
        "--fail",
        "--retry",
        "5",
        "--retry-delay",
        "2",
        "--continue-at",
        "-",
        "--output",
        str(part),
        url,
    ]
    print(f"downloading {target.name} ({entry['size']:,} bytes)")
    result = subprocess.run(command)
    if result.returncode != 0:
        raise SystemExit(f"download failed for {target.name}; resume with the same command")
    if part.stat().st_size != entry["size"]:
        raise SystemExit(
            f"size mismatch for {target.name}: "
            f"expected {entry['size']}, got {part.stat().st_size}"
        )
    actual = sha256(part)
    if actual != entry["sha256"]:
        raise SystemExit(
            f"SHA-256 mismatch for {target.name}: expected {entry['sha256']}, got {actual}"
        )
    part.replace(target)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dest", type=Path, default=default_dest())
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify existing files without downloading anything.",
    )
    args = parser.parse_args()
    manifest = load_manifest()
    dest: Path = args.dest.expanduser().resolve()
    files = manifest["files"]

    missing = [
        entry
        for entry in files
        if not is_valid(dest / entry["name"], entry)
    ]
    if args.check:
        if missing:
            for entry in missing:
                print(f"MISSING or INVALID: {dest / entry['name']}")
            return 1
        print(f"OK: all {len(files)} pinned MADLAD files are present in {dest}")
        return 0

    dest.mkdir(parents=True, exist_ok=True)
    needed = sum(entry["size"] for entry in missing)
    free = shutil.disk_usage(dest).free
    print(f"destination: {dest}")
    print(f"missing: {len(missing)} file(s), {needed:,} bytes")
    print(f"free disk: {free:,} bytes")
    if needed + MARGIN > free:
        raise SystemExit("not enough free disk space; nothing was downloaded")

    if not shutil.which("curl"):
        raise SystemExit("curl is required for resumable downloads")
    for entry in missing:
        target = dest / entry["name"]
        if target.exists() and not is_valid(target, entry):
            # Keep the unexpected file for inspection instead of deleting it.
            backup = target.with_name(target.name + ".invalid")
            target.replace(backup)
            print(f"moved invalid {target.name} to {backup.name}")
        url = BASE_URL.format(
            repo=manifest["repository"],
            revision=manifest["revision"],
            name=entry["name"],
        )
        download(url, target, entry)

    for entry in files:
        if not is_valid(dest / entry["name"], entry):
            raise SystemExit(f"final verification failed for {entry['name']}")
    print(f"OK: verified {len(files)} files in {dest}")
    print("License: Apache-2.0 (model card google/madlad400-3b-mt)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
