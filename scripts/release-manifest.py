#!/usr/bin/env python3
"""Generate a release manifest from the actual artifact files.

The manifest is derived from the bytes on disk: names, sizes, SHA-256 and
signature presence. Nothing is hardcoded except build metadata supplied by CI.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def classify(name: str) -> tuple[str, str, str]:
    if "-linux-" in name:
        platform, rest = "linux", name.split("-linux-", 1)[1]
        fmt = "tar.gz" if rest.endswith(".tar.gz") else "zip"
        arch = rest.removesuffix(".tar.gz").removesuffix(".zip")
        return platform, arch, fmt
    if "-windows-" in name:
        rest = name.split("-windows-", 1)[1]
        return "windows", rest.removesuffix(".zip"), "zip"
    if "-macos-" in name:
        rest = name.split("-macos-", 1)[1]
        return "macos", rest.removesuffix(".zip"), "zip"
    return "unknown", "unknown", "unknown"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dir", default="dist")
    parser.add_argument("--output", default=None)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", default="")
    parser.add_argument("--run-id", default="")
    parser.add_argument("--workflow", default="")
    parser.add_argument("--glibc-baseline", default="2.35")
    args = parser.parse_args()

    directory = Path(args.dir)
    output = Path(args.output) if args.output else directory / f"release-manifest-{args.version}.json"
    archives = sorted(
        path
        for path in directory.iterdir()
        if path.is_file()
        and (path.name.endswith(".tar.gz") or path.name.endswith(".zip"))
        and not path.name.endswith(".sha256")
        and "source" not in path.name
    )
    if not archives:
        raise SystemExit(f"no distribution archives found in {directory}")

    artifacts = []
    for archive in archives:
        platform, arch, fmt = classify(archive.name)
        signature = Path(str(archive) + ".minisig")
        checksum = Path(str(archive) + ".sha256")
        artifacts.append(
            {
                "filename": archive.name,
                "platform": platform,
                "architecture": arch,
                "format": fmt,
                "size_bytes": archive.stat().st_size,
                "sha256": sha256(archive),
                "checksum_file": checksum.name if checksum.is_file() else None,
                "signature_file": signature.name if signature.is_file() else None,
                "signed": signature.is_file(),
                **(
                    {"minimum_glibc": args.glibc_baseline}
                    if platform == "linux"
                    else {}
                ),
            }
        )

    signed = sum(1 for artifact in artifacts if artifact["signed"])
    status = "signed" if signed == len(artifacts) else "unsigned" if signed == 0 else "partial"
    manifest = {
        "application": "Fluent",
        "version": args.version,
        "release_status": "unreleased-workflow-artifact",
        "git_commit": args.commit or None,
        "workflow_run": args.run_id or None,
        "workflow": args.workflow or None,
        "generated_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "signing": {"minisign_status": status, "signed_artifacts": signed,
                    "total_artifacts": len(artifacts)},
        "artifacts": artifacts,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(output)
    print(f"{status}: {signed}/{len(artifacts)} signed")


if __name__ == "__main__":
    main()
