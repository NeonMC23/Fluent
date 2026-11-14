#!/usr/bin/env python3
"""Independently verify Fluent distribution archives.

Checks are derived from the actual bytes, not from workflow declarations:

* expected filename, version, platform and architecture;
* adjacent SHA-256 sidecar and manifest consistency;
* exact archive member allow-list (no models, secrets or development files);
* executable format and CPU architecture;
* macOS bundle metadata;
* detached Minisign signatures against a supplied public key.

This script never downloads anything and never writes into the archives.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path, PurePosixPath

FORBIDDEN_NAMES = {
    "tokenizer.json",
    "config.json",
    "settings.json",
    ".env",
    ".git-credentials",
    "id_rsa",
    "id_ed25519",
}
FORBIDDEN_SUFFIXES = {
    ".bin",
    ".safetensors",
    ".gguf",
    ".pt",
    ".pth",
    ".onnx",
    ".key",
    ".pem",
    ".p12",
    ".pfx",
    ".minisig",
}
FORBIDDEN_PARTS = {"models", ".git", "target", "node_modules", "dist"}

LINUX_FILES = {"LICENSE", "README.txt", "bin/fluent"}
WINDOWS_FILES = {"LICENSE", "README.txt", "fluent.exe"}
MACOS_FILES = {
    "Fluent.app/Contents/Info.plist",
    "Fluent.app/Contents/MacOS/fluent",
    "Fluent.app/Contents/PkgInfo",
    "Fluent.app/Contents/Resources/Fluent.icns",
}

MACHINE_ELF = {"x86_64": 62, "aarch64": 183}
MACHINE_PE = {"x86_64": 0x8664}
CPU_MACHO = {"x86_64": 0x01000007, "arm64": 0x0100000C}


class VerificationError(Exception):
    pass


def fail(message: str) -> None:
    raise VerificationError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_member_name(name: str) -> None:
    if name.startswith("/") or name.startswith("\\"):
        fail(f"absolute path in archive: {name}")
    parts = PurePosixPath(name.replace("\\", "/")).parts
    if ".." in parts:
        fail(f"path traversal in archive: {name}")
    if any(part in FORBIDDEN_PARTS for part in parts):
        fail(f"development/model directory in archive: {name}")
    base = parts[-1].lower() if parts else ""
    if base in FORBIDDEN_NAMES or base.startswith("config.json"):
        fail(f"forbidden file in archive: {name}")
    if any(base.endswith(suffix) for suffix in FORBIDDEN_SUFFIXES):
        fail(f"forbidden file type in archive: {name}")


def check_elf(data: bytes, arch: str) -> None:
    if data[:4] != b"\x7fELF":
        fail("Linux executable is not an ELF file")
    if data[4] != 2:
        fail("Linux executable is not 64-bit")
    endian = "<" if data[5] == 1 else ">" if data[5] == 2 else None
    if endian is None:
        fail("Linux executable has an invalid endianness marker")
    machine = struct.unpack_from(f"{endian}H", data, 18)[0]
    expected = MACHINE_ELF.get(arch)
    if expected is None:
        fail(f"unsupported Linux architecture in filename: {arch}")
    if machine != expected:
        fail(f"ELF machine {machine} does not match filename architecture {arch}")


def check_pe(data: bytes, arch: str) -> None:
    if data[:2] != b"MZ":
        fail("Windows executable is not a PE/MZ file")
    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        fail("Windows executable has no PE signature")
    machine = struct.unpack_from("<H", data, pe_offset + 4)[0]
    expected = MACHINE_PE.get(arch)
    if expected is None:
        fail(f"unsupported Windows architecture in filename: {arch}")
    if machine != expected:
        fail(f"PE machine {machine:#x} does not match filename architecture {arch}")
    optional_magic = struct.unpack_from("<H", data, pe_offset + 24)[0]
    if optional_magic != 0x20B:
        fail("Windows executable is not PE32+")


def check_macho(data: bytes, arch: str) -> None:
    expected = CPU_MACHO.get(arch)
    if expected is None:
        fail(f"unsupported macOS architecture in filename: {arch}")

    if len(data) >= 8 and struct.unpack_from(">I", data, 0)[0] == 0xCAFEBABE:
        count = struct.unpack_from(">I", data, 4)[0]
        if count > 32:
            fail("macOS universal binary declares an implausible architecture count")
        found = {
            struct.unpack_from(">I", data, 8 + index * 20)[0]
            for index in range(count)
            if 8 + index * 20 + 4 <= len(data)
        }
        if expected not in found:
            fail(f"universal binary does not contain {arch}")
        if not found <= {0x01000007, 0x0100000C}:
            fail(f"universal binary contains unexpected CPU types: {found}")
        return

    if len(data) < 8 or struct.unpack_from("<I", data, 0)[0] != 0xFEEDFACF:
        fail("macOS executable is not a 64-bit thin Mach-O file")
    cputype = struct.unpack_from("<I", data, 4)[0]
    if cputype != expected:
        fail(f"Mach-O CPU type {cputype:#x} does not match filename architecture {arch}")


def inspect_linux(archive: Path, version: str, arch: str) -> None:
    root = f"fluent-{version}-linux-{arch}"
    with tarfile.open(archive, "r:gz") as handle:
        members = handle.getmembers()
        file_names: set[str] = set()
        dir_names: set[str] = set()
        for member in members:
            check_member_name(member.name)
            if not (member.name == root or member.name.startswith(root + "/")):
                fail(f"unexpected top-level entry: {member.name}")
            if member.issym() or member.islnk():
                fail(f"links are not allowed in the archive: {member.name}")
            if member.isfile():
                file_names.add(member.name)
            elif member.isdir():
                dir_names.add(member.name)
            else:
                fail(f"unexpected special entry: {member.name}")

        expected = {f"{root}/{name}" for name in LINUX_FILES}
        if file_names != expected:
            fail(
                "Linux archive contents differ: expected "
                f"{sorted(expected)}, found {sorted(file_names)}"
            )
        allowed_dirs = {root, f"{root}/bin"}
        if not dir_names <= allowed_dirs:
            fail(f"unexpected directories: {sorted(dir_names - allowed_dirs)}")

        binary = handle.extractfile(f"{root}/bin/fluent")
        if binary is None:
            fail("Linux executable is not readable from the archive")
        check_elf(binary.read(), arch)

        readme = handle.extractfile(f"{root}/README.txt")
        if readme is None:
            fail("Linux README is not readable")
        text = readme.read().decode("utf-8", errors="replace")
        if f"Fluent {version}" not in text:
            fail("Linux README does not identify the packaged version")


def inspect_windows(archive: Path, version: str, arch: str) -> None:
    with zipfile.ZipFile(archive) as handle:
        names = set()
        for info in handle.infolist():
            check_member_name(info.filename)
            if info.is_dir():
                continue
            names.add(info.filename)
        if names != WINDOWS_FILES:
            fail(
                "Windows archive contents differ: expected "
                f"{sorted(WINDOWS_FILES)}, found {sorted(names)}"
            )
        check_pe(handle.read("fluent.exe"), arch)
        readme = handle.read("README.txt").decode("utf-8", errors="replace")
        if f"Fluent {version}" not in readme:
            fail("Windows README does not identify the packaged version")


def inspect_macos(archive: Path, version: str, arch: str) -> None:
    with zipfile.ZipFile(archive) as handle:
        names = set()
        for info in handle.infolist():
            check_member_name(info.filename)
            if info.is_dir():
                continue
            names.add(info.filename)
        if names != MACOS_FILES:
            fail(
                "macOS archive contents differ: expected "
                f"{sorted(MACOS_FILES)}, found {sorted(names)}"
            )
        check_macho(handle.read("Fluent.app/Contents/MacOS/fluent"), arch)
        plist = plistlib.loads(handle.read("Fluent.app/Contents/Info.plist"))
        if plist.get("CFBundleShortVersionString") != version:
            fail("Info.plist CFBundleShortVersionString does not match the version")
        if plist.get("CFBundleVersion") != version:
            fail("Info.plist CFBundleVersion does not match the version")
        if plist.get("CFBundleIdentifier") != "io.fluent.Fluent":
            fail("unexpected CFBundleIdentifier")
        if plist.get("CFBundleExecutable") != "fluent":
            fail("unexpected CFBundleExecutable")
        if plist.get("CFBundleIconFile") != "Fluent.icns":
            fail("unexpected CFBundleIconFile")


def verify_manifest(
    directory: Path,
    version: str,
    archives: list[Path],
    signed: dict[Path, bool],
) -> None:
    path = directory / "release-manifest.json"
    if not path.is_file():
        print("manifest: not present (local build)")
        return
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("application") != "Fluent":
        fail("manifest application is not Fluent")
    if document.get("version") != version:
        fail("manifest version does not match the requested version")
    if not document.get("git_commit"):
        fail("manifest has no git commit")
    artifacts = document.get("artifacts")
    if not isinstance(artifacts, list):
        fail("manifest artifacts is not a list")
    by_name = {entry.get("filename"): entry for entry in artifacts}
    if len(by_name) != len(archives):
        fail("manifest does not list every archive exactly once")
    for archive in archives:
        entry = by_name.get(archive.name)
        if entry is None:
            fail(f"manifest is missing {archive.name}")
        if entry.get("size_bytes") != archive.stat().st_size:
            fail(f"manifest size mismatch for {archive.name}")
        if entry.get("sha256") != sha256(archive):
            fail(f"manifest SHA-256 mismatch for {archive.name}")
        if entry.get("signed") != signed[archive]:
            fail(f"manifest signature status mismatch for {archive.name}")
    signed_count = sum(1 for value in signed.values() if value)
    status = (
        "signed"
        if signed_count == len(archives)
        else "unsigned"
        if signed_count == 0
        else "partial"
    )
    signing = document.get("signing") or {}
    if signing.get("minisign_status") != status:
        fail("manifest Minisign status does not match the archives")
    if signing.get("signed_artifacts") != signed_count:
        fail("manifest signed artifact count does not match")
    print(
        "manifest: OK "
        f"(commit {str(document.get('git_commit'))[:12]}, signing {status}, "
        f"verified={signing.get('signatures_verified')})"
    )


def verify_signature(archive: Path, public_key: Path) -> None:
    signature = Path(str(archive) + ".minisig")
    if shutil.which("minisign"):
        command = [
            "minisign",
            "-V",
            "-p",
            str(public_key),
            "-m",
            str(archive),
            "-x",
            str(signature),
        ]
    elif shutil.which("rsign"):
        command = [
            "rsign",
            "verify",
            "-p",
            str(public_key),
            "-x",
            str(signature),
            str(archive),
        ]
    else:
        fail("signature exists but neither minisign nor rsign is installed")
    result = subprocess.run(command, text=True, capture_output=True)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().splitlines()
        fail(f"signature verification failed: {detail[0] if detail else 'unknown error'}")
    print(f"signature: OK ({archive.name})")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", default="dist", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--public-key", type=Path)
    parser.add_argument(
        "--require-signatures",
        action="store_true",
        help="Fail unless every archive has a valid detached signature.",
    )
    args = parser.parse_args()

    directory = args.dir
    if not directory.is_dir():
        print(f"FAIL: not a directory: {directory}", file=sys.stderr)
        return 2

    archives = sorted(
        path
        for path in directory.iterdir()
        if path.is_file()
        and (path.name.endswith(".tar.gz") or path.name.endswith(".zip"))
        and "source" not in path.name
    )
    if not archives:
        print(f"FAIL: no distribution archives in {directory}", file=sys.stderr)
        return 2

    public_key = args.public_key
    if public_key is None:
        candidate = directory / "minisign.pub"
        if candidate.is_file():
            public_key = candidate
    if public_key is not None and not public_key.is_file():
        print(f"FAIL: public key not found: {public_key}", file=sys.stderr)
        return 2

    signed: dict[Path, bool] = {}
    failures: list[str] = []

    for archive in archives:
        try:
            match = re.fullmatch(
                rf"fluent-{re.escape(args.version)}-linux-(x86_64|aarch64)\.tar\.gz",
                archive.name,
            )
            if match:
                platform, arch = "linux", match.group(1)
                inspect_linux(archive, args.version, arch)
            else:
                match = re.fullmatch(
                    rf"fluent-{re.escape(args.version)}-windows-(x86_64)\.zip",
                    archive.name,
                )
                if match:
                    platform, arch = "windows", match.group(1)
                    inspect_windows(archive, args.version, arch)
                else:
                    match = re.fullmatch(
                        rf"Fluent-{re.escape(args.version)}-macos-(arm64|x86_64)\.zip",
                        archive.name,
                    )
                    if not match:
                        fail("unrecognised distribution archive name")
                    platform, arch = "macos", match.group(1)
                    inspect_macos(archive, args.version, arch)

            sidecar = Path(str(archive) + ".sha256")
            if not sidecar.is_file():
                fail("missing SHA-256 sidecar")
            fields = sidecar.read_text(encoding="utf-8").strip().split()
            if len(fields) != 2 or not re.fullmatch(r"[0-9a-fA-F]{64}", fields[0]):
                fail("malformed SHA-256 sidecar")
            if fields[1] != archive.name:
                fail("SHA-256 sidecar names a different archive")
            actual = sha256(archive)
            if fields[0].lower() != actual:
                fail("SHA-256 sidecar does not match the archive")
            print(
                f"{archive.name}: OK ({platform}/{arch}, "
                f"{archive.stat().st_size} bytes, sha256 {actual})"
            )

            signature = Path(str(archive) + ".minisig")
            signed[archive] = signature.is_file()
            if signature.is_file():
                if public_key is None:
                    fail("signature is present but no public key was supplied")
                verify_signature(archive, public_key)
            else:
                print(f"signature: UNSIGNED ({archive.name})")
                if args.require_signatures:
                    fail("signature is required but missing")
        except VerificationError as error:
            failures.append(f"{archive.name}: {error}")
            print(f"FAIL: {archive.name}: {error}", file=sys.stderr)

    try:
        verify_manifest(directory, args.version, archives, signed)
    except VerificationError as error:
        failures.append(f"manifest: {error}")
        print(f"FAIL: manifest: {error}", file=sys.stderr)

    if failures:
        print(f"\nFAILED: {len(failures)} verification problem(s)", file=sys.stderr)
        return 1

    print("\nARTIFACT VERIFICATION OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
