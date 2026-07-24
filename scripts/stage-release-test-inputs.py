#!/usr/bin/env python3
"""Stage verified release inputs for the shared functional test modules."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urljoin, urlparse


def _host_arch() -> str:
    machine = platform.machine().lower()
    if machine in {"arm64", "aarch64"}:
        return "arm64"
    if machine in {"x86_64", "amd64"}:
        return "x86_64"
    raise ValueError(f"unsupported host architecture: {machine}")


def _load(input_dir: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    report = json.loads((input_dir / "release-inputs.json").read_text(encoding="utf-8"))
    manifest = json.loads((input_dir / "manifest.json").read_text(encoding="utf-8"))
    if report.get("schema") != "capsem.release_inputs.v1":
        raise ValueError("release input report has an unsupported schema")
    if not isinstance(manifest, dict):
        raise ValueError("release manifest must be an object")
    return report, manifest


def _local_url_map(input_dir: Path, report: dict[str, Any]) -> dict[str, str]:
    result: dict[str, str] = {}
    for row in report.get("artifacts", []):
        if not isinstance(row, dict):
            raise ValueError("release input artifact row is malformed")
        url = row.get("url")
        relative = row.get("path")
        if not isinstance(url, str) or not isinstance(relative, str):
            raise ValueError("release input artifact row lacks URL or path")
        result[url] = (input_dir / relative).resolve().as_uri()
    return result


def _rewrite_urls(value: Any, replacements: dict[str, str], manifest_url: str) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            absolute = urljoin(manifest_url, child) if isinstance(child, str) else ""
            if key == "url" and absolute in replacements:
                value[key] = replacements[absolute]
            else:
                _rewrite_urls(child, replacements, manifest_url)
    elif isinstance(value, list):
        for child in value:
            _rewrite_urls(child, replacements, manifest_url)


def stage_profiles(input_dir: Path, assets_dir: Path) -> Path:
    report, manifest = _load(input_dir)
    if report.get("kind") != "profiles":
        raise ValueError("profile staging requires profile release inputs")
    replacements = _local_url_map(input_dir, report)
    manifest_url = report.get("manifest_url")
    if not isinstance(manifest_url, str):
        raise ValueError("release input report lacks its manifest URL")
    _rewrite_urls(manifest, replacements, manifest_url)
    assets_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = assets_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    arch = _host_arch()
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("release manifest contains no profiles")
    selected: dict[str, Path] = {}
    for profile in profiles.values():
        if not isinstance(profile, dict):
            continue
        for architecture in profile.get("architectures", []):
            if not isinstance(architecture, dict) or architecture.get("architecture") != arch:
                continue
            for image in architecture.get("images", []):
                if not isinstance(image, dict):
                    continue
                kind = image.get("kind")
                url = image.get("url")
                if kind in {"kernel", "initrd", "rootfs"} and isinstance(url, str):
                    parsed = urlparse(url)
                    if parsed.scheme != "file":
                        continue
                    path = Path(unquote(parsed.path))
                    selected.setdefault(kind, path)
    names = {"kernel": "vmlinuz", "initrd": "initrd.img", "rootfs": "rootfs.erofs"}
    missing = set(names) - set(selected)
    if missing:
        raise ValueError(f"release profiles lack host-architecture images: {sorted(missing)}")
    arch_dir = assets_dir / arch
    arch_dir.mkdir(parents=True, exist_ok=True)
    for kind, name in names.items():
        shutil.copy2(selected[kind], arch_dir / name)
    return manifest_path


def stage_package_binaries(input_dir: Path, binary_dir: Path) -> list[Path]:
    report, manifest = _load(input_dir)
    if report.get("kind") != "packages":
        raise ValueError("package staging requires package release inputs")
    packages = manifest.get("packages")
    if not isinstance(packages, list):
        raise ValueError("release manifest packages are malformed")
    host_arch = "arm64" if _host_arch() == "arm64" else "amd64"
    url_to_path = {
        row["url"]: input_dir / row["path"]
        for row in report.get("artifacts", [])
        if isinstance(row, dict)
        and isinstance(row.get("url"), str)
        and isinstance(row.get("path"), str)
    }
    candidates = [
        package
        for package in packages
        if isinstance(package, dict)
        and package.get("status") == "current"
        and package.get("platform") == "linux"
        and package.get("architecture") == host_arch
        and package.get("url") in url_to_path
    ]
    if len(candidates) != 1:
        raise ValueError(
            f"expected one current Linux {host_arch} package, found {len(candidates)}"
        )
    package_path = url_to_path[candidates[0]["url"]]
    extract_dir = binary_dir.parent / "resolved-package"
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)
    subprocess.run(
        ("dpkg-deb", "--extract", str(package_path), str(extract_dir)),
        check=True,
    )
    source_dir = extract_dir / "usr" / "bin"
    binaries = sorted(source_dir.glob("capsem*"))
    if not binaries:
        raise ValueError(f"package {package_path} contains no Capsem host binaries")
    binary_dir.mkdir(parents=True, exist_ok=True)
    staged = []
    for source in binaries:
        destination = binary_dir / source.name
        shutil.copy2(source, destination)
        os.chmod(destination, 0o755)
        staged.append(destination)
    return staged


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", required=True, type=Path)
    parser.add_argument("--assets-dir", type=Path, default=Path("assets"))
    parser.add_argument("--binary-dir", type=Path, default=Path("target/debug"))
    args = parser.parse_args()
    try:
        report, _ = _load(args.input_dir)
        if report.get("kind") == "profiles":
            result = [stage_profiles(args.input_dir, args.assets_dir)]
        elif report.get("kind") == "packages":
            result = stage_package_binaries(args.input_dir, args.binary_dir)
        else:
            raise ValueError("release input report has an invalid artifact kind")
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"release input staging failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"staged": [str(path) for path in result]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
