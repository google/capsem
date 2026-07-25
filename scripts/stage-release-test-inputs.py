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
from typing import Any, cast
from urllib.parse import unquote, urljoin, urlparse

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

from release_inputs import (  # noqa: E402
    load_verified_release_inputs,
    safe_component,
    safe_relative,
    verify_payload,
)


def _host_arch() -> str:
    machine = platform.machine().lower()
    if machine in {"arm64", "aarch64"}:
        return "arm64"
    if machine in {"x86_64", "amd64"}:
        return "x86_64"
    raise ValueError(f"unsupported host architecture: {machine}")


def _load(input_dir: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    report, manifest, _ = load_verified_release_inputs(input_dir)
    return report, manifest


def _local_url_map(input_dir: Path, report: dict[str, Any]) -> dict[str, str]:
    result: dict[str, str] = {}
    for row in report.get("artifacts", []):
        if not isinstance(row, dict):
            raise ValueError("release input artifact row is malformed")
        url = row.get("url")
        relative = row.get("path")
        if not isinstance(url, str):
            raise ValueError("release input artifact row lacks URL or path")
        local = safe_relative(relative)
        result[url] = (input_dir / local).resolve().as_uri()
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


def _local_file(url: object, label: str) -> Path:
    if not isinstance(url, str):
        raise ValueError(f"{label} lacks a staged URL")
    parsed = urlparse(url)
    if parsed.scheme != "file":
        raise ValueError(f"{label} was not resolved to a local immutable input")
    return Path(unquote(parsed.path))


def _hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, extension = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{extension}"
    return f"{logical_name}-{prefix}"


def _reset_staging_directory(path: Path, label: str) -> None:
    resolved = path.resolve()
    if resolved in {Path("/"), Path.home().resolve()}:
        raise ValueError(f"refusing to replace broad {label} directory {resolved}")
    if path.exists():
        if path.is_symlink() or not path.is_dir():
            raise ValueError(f"{label} staging path must be a real directory: {path}")
        shutil.rmtree(path)
    path.mkdir(parents=True)


def _active_profile_architectures(
    manifest: dict[str, Any], arch: str
) -> list[tuple[str, dict[str, Any]]]:
    profiles = manifest.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        raise ValueError("release manifest contains no profiles")
    selected: list[tuple[str, dict[str, Any]]] = []
    for profile_id_value, profile in sorted(profiles.items()):
        profile_id = safe_component(profile_id_value, "profile identity")
        if not isinstance(profile, dict):
            raise ValueError(f"release profile {profile_id} is malformed")
        profile = cast(dict[str, Any], profile)
        if profile.get("status") == "revoked":
            continue
        architectures = [
            candidate
            for candidate in profile.get("architectures", [])
            if isinstance(candidate, dict) and candidate.get("architecture") == arch
        ]
        if len(architectures) != 1:
            raise ValueError(
                f"release profile {profile_id} must have exactly one {arch} architecture"
            )
        selected.append((profile_id, architectures[0]))
    if not selected:
        raise ValueError(f"release manifest contains no active {arch} profiles")
    selected.sort(key=lambda row: (row[0] != "code", row[0]))
    return selected


def stage_profiles(
    input_dir: Path,
    assets_dir: Path,
    config_root: Path = Path("target/release-config"),
) -> Path:
    report, manifest = _load(input_dir)
    if report.get("kind") != "profiles":
        raise ValueError("profile staging requires profile release inputs")
    host_arch = _host_arch()
    selected_arch = report.get("architecture")
    if selected_arch is not None and selected_arch != host_arch:
        raise ValueError(f"profile release inputs select {selected_arch}, not host {host_arch}")
    replacements = _local_url_map(input_dir, report)
    manifest_url = report.get("manifest_url")
    if not isinstance(manifest_url, str):
        raise ValueError("release input report lacks its manifest URL")
    _rewrite_urls(manifest, replacements, manifest_url)
    if assets_dir.resolve() == config_root.resolve():
        raise ValueError("profile assets and config staging roots must differ")
    _reset_staging_directory(assets_dir, "profile asset")
    _reset_staging_directory(config_root, "profile config")
    manifest_path = assets_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    arch = host_arch
    arch_dir = assets_dir / arch
    arch_dir.mkdir(parents=True, exist_ok=True)
    logical_names = {
        "kernel": "vmlinuz",
        "initrd": "initrd.img",
        "rootfs": "rootfs.erofs",
    }
    for profile_index, (profile_id, architecture) in enumerate(
        _active_profile_architectures(manifest, arch)
    ):
        configs = architecture.get("config")
        if not isinstance(configs, list) or not configs:
            raise ValueError(f"release profile {profile_id}/{arch} has no config")
        staged_config_paths: set[Path] = set()
        for index, record in enumerate(configs):
            if not isinstance(record, dict):
                raise ValueError(
                    f"release profile {profile_id}/{arch} config[{index}] is malformed"
                )
            record = cast(dict[str, Any], record)
            if record.get("status") == "revoked":
                continue
            relative = safe_relative(
                record.get("path"),
                f"profile {profile_id}/{arch} config[{index}] path",
            )
            if len(relative.parts) < 3 or relative.parts[:2] != (
                "profiles",
                profile_id,
            ):
                raise ValueError(
                    f"profile {profile_id}/{arch} config path escapes its profile: {relative}"
                )
            if relative in staged_config_paths:
                raise ValueError(f"profile {profile_id}/{arch} repeats config path {relative}")
            staged_config_paths.add(relative)
            source = _local_file(record.get("url"), f"profile {profile_id}/{arch} config[{index}]")
            destination = config_root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        expected_profile = Path("profiles") / profile_id / "profile.toml"
        if expected_profile not in staged_config_paths:
            raise ValueError(f"release profile {profile_id}/{arch} lacks {expected_profile}")

        images = architecture.get("images")
        if not isinstance(images, list):
            raise ValueError(f"release profile {profile_id}/{arch} images are malformed")
        by_kind: dict[str, tuple[str, str, Path]] = {}
        for index, record in enumerate(images):
            if not isinstance(record, dict):
                raise ValueError(f"release profile {profile_id}/{arch} image[{index}] is malformed")
            record = cast(dict[str, Any], record)
            if record.get("status") == "revoked":
                continue
            kind = record.get("kind")
            if kind not in logical_names:
                continue
            if kind in by_kind:
                raise ValueError(f"release profile {profile_id}/{arch} repeats {kind} image")
            logical_name = safe_component(
                record.get("name") or logical_names[kind],
                f"profile {profile_id}/{arch} {kind} image name",
            )
            digest = record.get("digest", {}).get("blake3")
            if not isinstance(digest, str):
                raise ValueError(f"release profile {profile_id}/{arch} {kind} lacks BLAKE3")
            source = _local_file(record.get("url"), f"profile {profile_id}/{arch} image[{index}]")
            by_kind[kind] = (logical_name, digest, source)
        missing = set(logical_names) - set(by_kind)
        if missing:
            raise ValueError(f"release profile {profile_id}/{arch} lacks images: {sorted(missing)}")
        for kind, (logical_name, digest, source) in by_kind.items():
            hashed = arch_dir / _hash_filename(logical_name, digest)
            if not hashed.exists():
                shutil.copy2(source, hashed)
            elif hashed.read_bytes() != source.read_bytes():
                raise ValueError(f"release profiles collide at immutable image {hashed.name}")
            # A few fail-fast checks still require the legacy logical names.
            # The first deterministic active profile supplies those aliases;
            # every VM boot resolves its selected profile by the hash name.
            if profile_index == 0:
                shutil.copy2(source, arch_dir / logical_names[kind])
    return manifest_path


def _select_host_package(
    input_dir: Path,
) -> tuple[dict[str, Any], Path]:
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
        and urljoin(str(report["manifest_url"]), str(package.get("url"))) in url_to_path
    ]
    if len(candidates) != 1:
        raise ValueError(f"expected one current Linux {host_arch} package, found {len(candidates)}")
    package = candidates[0]
    package_url = urljoin(str(report["manifest_url"]), str(package["url"]))
    return package, url_to_path[package_url]


def select_host_package_path(input_dir: Path) -> Path:
    return _select_host_package(input_dir)[1]


def stage_package_binaries(input_dir: Path, binary_dir: Path) -> list[Path]:
    package, package_path = _select_host_package(input_dir)
    extract_dir = binary_dir.parent / "resolved-package"
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)
    subprocess.run(
        ("dpkg-deb", "--extract", str(package_path), str(extract_dir)),
        check=True,
    )
    inventory = package.get("binaries")
    if not isinstance(inventory, list) or not inventory:
        raise ValueError(f"package {package_path} has no host binary inventory")
    binaries: list[Path] = []
    expected_names: set[str] = set()
    for index, record in enumerate(inventory):
        if not isinstance(record, dict):
            raise ValueError(f"package binary[{index}] inventory row is malformed")
        record = cast(dict[str, Any], record)
        if record.get("status") == "revoked":
            continue
        name = safe_component(record.get("name"), f"package binary[{index}] inventory name")
        installed_path = record.get("installed_path")
        if not isinstance(installed_path, str) or not installed_path.startswith("/usr/bin/"):
            raise ValueError(
                f"package binary {name} has unsupported installed path {installed_path!r}"
            )
        relative = safe_relative(
            installed_path.removeprefix("/"),
            f"package binary {name} installed path",
        )
        if relative.name != name:
            raise ValueError(
                f"package binary {name} does not match installed path {installed_path}"
            )
        if name in expected_names:
            raise ValueError(f"package binary inventory repeats {name}")
        expected_names.add(name)
        source = extract_dir / relative
        try:
            payload = source.read_bytes()
        except OSError as error:
            raise ValueError(f"package {package_path} lacks inventoried binary {name}") from error
        verify_payload(payload, record, f"package binary {name}")
        binaries.append(source)
    actual_names = {
        path.name for path in (extract_dir / "usr/bin").glob("capsem*") if path.is_file()
    }
    if actual_names != expected_names:
        raise ValueError(
            "package host binary inventory mismatch: "
            f"expected {sorted(expected_names)}, found {sorted(actual_names)}"
        )

    binary_dir.mkdir(parents=True, exist_ok=True)
    for stale in binary_dir.glob("capsem*"):
        if stale.is_dir() and not stale.is_symlink():
            raise ValueError(f"refusing to replace unexpected binary directory {stale}")
        stale.unlink()
    staged: list[Path] = []
    for source in binaries:
        destination = binary_dir / source.name
        shutil.copy2(source, destination)
        os.chmod(destination, 0o755)
        staged.append(destination)
    return staged


def stage_candidate_package(package_path: Path, binary_dir: Path) -> list[Path]:
    extract_dir = binary_dir.parent / "candidate-package"
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)
    subprocess.run(
        ("dpkg-deb", "--extract", str(package_path), str(extract_dir)),
        check=True,
    )
    binaries = sorted(path for path in (extract_dir / "usr/bin").glob("capsem*") if path.is_file())
    if not binaries:
        raise ValueError(f"candidate package {package_path} contains no Capsem binaries")
    binary_dir.mkdir(parents=True, exist_ok=True)
    for stale in binary_dir.glob("capsem*"):
        if stale.is_dir() and not stale.is_symlink():
            raise ValueError(f"refusing to replace unexpected binary directory {stale}")
        stale.unlink()
    staged = []
    for source in binaries:
        destination = binary_dir / source.name
        shutil.copy2(source, destination)
        os.chmod(destination, 0o755)
        staged.append(destination)
    return staged


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--input-dir", type=Path)
    source.add_argument("--package-file", type=Path)
    parser.add_argument("--assets-dir", type=Path, default=Path("assets"))
    parser.add_argument("--binary-dir", type=Path, default=Path("target/debug"))
    parser.add_argument(
        "--config-root",
        type=Path,
        default=Path("target/release-config"),
    )
    parser.add_argument("--print-package-path", action="store_true")
    args = parser.parse_args()
    try:
        if args.print_package_path:
            if args.input_dir is None:
                raise ValueError("--print-package-path requires --input-dir")
            print(select_host_package_path(args.input_dir))
            return 0
        if args.package_file is not None:
            result = stage_candidate_package(args.package_file, args.binary_dir)
        else:
            report, _ = _load(args.input_dir)
            if report.get("kind") == "profiles":
                result = [
                    stage_profiles(
                        args.input_dir,
                        args.assets_dir,
                        args.config_root,
                    )
                ]
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
