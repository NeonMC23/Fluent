#!/usr/bin/env python3
"""Configure CARGO_ENCODED_RUSTFLAGS for their CI build environment.

Rust panic locations otherwise embed the runner workspace and registry paths.
The encoded form separates arguments with 0x1f, so paths containing spaces are
safe. Run only in CI; it appends to $GITHUB_ENV for subsequent steps.
"""

from __future__ import annotations

import os
from pathlib import Path

workspace = Path(os.environ["GITHUB_WORKSPACE"])
cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
target = Path(os.environ.get("CARGO_TARGET_DIR", str(workspace / "target")))
flags = [
    f"--remap-path-prefix={workspace}=.",
    f"--remap-path-prefix={cargo_home / 'registry'}=/cargo/registry",
    f"--remap-path-prefix={target}=/target",
]
with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env:
    env.write("CARGO_ENCODED_RUSTFLAGS=" + "\x1f".join(flags) + "\n")
print("Configured CI path remapping")
