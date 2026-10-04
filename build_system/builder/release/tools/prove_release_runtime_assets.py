"""Boot one manifest-selected profile's verified image bytes without building artifacts."""

from __future__ import annotations

import argparse
import platform
import subprocess
import sys
from pathlib import Path
from typing import Any, cast
from urllib.parse import urljoin

from .release_inputs import (
    load_verified_release_inputs,
    required_digest,
    safe_component,
    safe_relative,
)


def host_architecture() -> str:
    machine = platform.machine().lower()
    if machine in {"arm64", "aarch64"}:
        return "arm64"
    if machine in {"x86_64", "amd64"}:
        return "x86_64"
    raise ValueError(f"unsupported host architecture: {machine}")


def resolve_profile_boot_inputs(
    input_dir: Path,
    profile_id: str,
    architecture: str,
) -> dict[str, tuple[Path, str]]:
    profile_id = safe_component(profile_id, "profile identity")
    architecture = safe_component(architecture, "profile architecture")
    report, manifest, _ = load_verified_release_inputs(input_dir)
    if report.get("kind") != "profiles":
        raise ValueError("profile boot proof requires profile release inputs")
    selected_architecture = report.get("architecture")
    if selected_architecture is not None and selected_architecture != architecture:
        raise ValueError(
            "profile release inputs select "
            f"{selected_architecture}, not host {architecture}"
        )

    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or profile_id not in profiles:
        raise ValueError(f"manifest does not select profile {profile_id}")
    profile = profiles[profile_id]
    if not isinstance(profile, dict) or profile.get("status") == "revoked":
        raise ValueError(f"manifest profile {profile_id} is not active")
    architectures = profile.get("architectures")
    if not isinstance(architectures, list):
        raise ValueError(f"manifest profile {profile_id} architectures are malformed")
    matches = [
        candidate
        for candidate in architectures
        if isinstance(candidate, dict)
        and candidate.get("architecture") == architecture
    ]
    if len(matches) != 1:
        raise ValueError(
            f"manifest profile {profile_id} must select exactly one {architecture} architecture"
        )

    manifest_url = report.get("manifest_url")
    if not isinstance(manifest_url, str) or not manifest_url:
        raise ValueError("release input report lacks its manifest URL")
    local_by_url: dict[str, Path] = {}
    for row in report.get("artifacts", []):
        if not isinstance(row, dict):
            raise ValueError("release input artifact row is malformed")
        url = row.get("url")
        relative = row.get("path")
        if not isinstance(url, str):
            raise ValueError("release input artifact row lacks a URL")
        local_by_url[url] = input_dir / safe_relative(relative)

    images = matches[0].get("images")
    if not isinstance(images, list):
        raise ValueError(f"manifest profile {profile_id}/{architecture} images are malformed")
    selected: dict[str, tuple[Path, str]] = {}
    for index, value in enumerate(images):
        if not isinstance(value, dict):
            raise ValueError(
                f"manifest profile {profile_id}/{architecture} image[{index}] is malformed"
            )
        record = cast(dict[str, Any], value)
        if record.get("status") == "revoked":
            continue
        kind = record.get("kind")
        if kind not in {"kernel", "initrd", "rootfs"}:
            continue
        if kind in selected:
            raise ValueError(
                f"manifest profile {profile_id}/{architecture} repeats {kind} image"
            )
        url = record.get("url")
        if not isinstance(url, str) or not url:
            raise ValueError(
                f"manifest profile {profile_id}/{architecture} {kind} has no URL"
            )
        absolute = urljoin(manifest_url, url)
        local = local_by_url.get(absolute)
        if local is None or not local.is_file():
            raise ValueError(
                f"verified inputs lack profile {profile_id}/{architecture} {kind}"
            )
        _, blake3, _ = required_digest(
            record,
            f"profile {profile_id}/{architecture} {kind}",
        )
        selected[kind] = (local, blake3)

    missing = {"kernel", "initrd", "rootfs"} - set(selected)
    if missing:
        raise ValueError(
            f"manifest profile {profile_id}/{architecture} lacks boot images: {sorted(missing)}"
        )
    return selected


def prove_profile_assets(
    input_dir: Path,
    profile_id: str,
    *,
    architecture: str | None = None,
    timeout: int = 300,
    runner: Any = subprocess.run,
) -> None:
    if timeout <= 0:
        raise ValueError("profile asset boot timeout must be positive")
    selected_architecture = architecture or host_architecture()
    images = resolve_profile_boot_inputs(
        input_dir,
        profile_id,
        selected_architecture,
    )
    command = [
        "cargo",
        "run",
        "--locked",
        "-p",
        "capsem-core",
        "--example",
        "release_profile_boot",
        "--",
        "--profile",
        profile_id,
        "--kernel",
        str(images["kernel"][0]),
        "--kernel-blake3",
        images["kernel"][1],
        "--initrd",
        str(images["initrd"][0]),
        "--initrd-blake3",
        images["initrd"][1],
        "--rootfs",
        str(images["rootfs"][0]),
        "--rootfs-blake3",
        images["rootfs"][1],
        "--timeout",
        str(timeout),
    ]
    runner(command, check=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--architecture")
    parser.add_argument("--timeout", type=int, default=300)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        prove_profile_assets(
            args.input_dir,
            args.profile,
            architecture=args.architecture,
            timeout=args.timeout,
        )
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"release profile asset boot proof failed: {error}", file=sys.stderr)
        return 1
    print(f"release profile asset boot proof passed: {args.profile}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
