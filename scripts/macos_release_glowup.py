#!/usr/bin/env python3
"""Build and prove the exact macOS package without a Just recipe fork."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import platform
import re
import shlex
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    print("+", shlex.join(command), flush=True)
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def project_version() -> str:
    manifest = (ROOT / "Cargo.toml").read_text()
    workspace = re.search(
        r"(?ms)^\[workspace\.package\]\s*(.*?)(?=^\[|\Z)",
        manifest,
    )
    if workspace is None:
        raise RuntimeError("Cargo.toml is missing [workspace.package]")
    version = re.search(r'(?m)^version\s*=\s*"([^"]+)"\s*$', workspace.group(1))
    if version is None:
        raise RuntimeError("Cargo.toml [workspace.package] is missing version")
    return version.group(1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default=project_version())
    parser.add_argument(
        "--channel",
        choices=("stable", "nightly"),
        default=os.environ.get("CAPSEM_INSTALL_CHANNEL", "stable"),
    )
    parser.add_argument("--manifest-url")
    args = parser.parse_args()

    if platform.system() != "Darwin":
        raise RuntimeError("the macOS release glow-up requires macOS")
    manifest_url = args.manifest_url or os.environ.get(
        "CAPSEM_INSTALL_MANIFEST_URL",
        f"https://release.capsem.org/assets/{args.channel}/manifest.json",
    )

    frontend_env = os.environ.copy()
    frontend_env["CI"] = "true"
    run(
        ["pnpm", "--dir", "frontend", "install", "--frozen-lockfile"],
        env=frontend_env,
    )
    run(["bash", "scripts/materialize-config.sh"])
    run(
        [
            "bash",
            "scripts/build-test-macos-package.sh",
            "--version",
            args.version,
            "--manifest-url",
            manifest_url,
        ]
    )
    package = ROOT / "packages" / f"Capsem-{args.version}.pkg"
    run(
        [
            sys.executable,
            "scripts/macos_tart_glowup.py",
            "--package",
            str(package),
            "--version",
            args.version,
            "--manifest-url",
            manifest_url,
            "--channel",
            args.channel,
        ]
    )
    run(["bash", "scripts/prove-macos-package-boot.sh", str(package), args.version])
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"macOS release glow-up failed: {error}", file=sys.stderr)
        raise SystemExit(1)
