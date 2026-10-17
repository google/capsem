"""Stage verified release inputs for the shared functional test modules."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any, cast
from urllib.parse import unquote, urljoin, urlparse

from . import repository_root
from .package_payload import deb_payload_files
from .release_cohort import REQUIRED_LINUX_RELEASE_BINARIES
from .release_inputs import (
    load_verified_release_inputs,
    safe_component,
    safe_relative,
    verify_payload,
)

ROOT = repository_root()

#: The logical names the service boots from, by manifest image kind.
IMAGE_NAMES = {"kernel": "vmlinuz", "initrd": "initrd.img", "rootfs": "rootfs.erofs"}


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


def _reset_staging_directory(path: Path, label: str) -> None:
    resolved = path.resolve()
    if resolved in {Path("/"), Path.home().resolve()}:
        raise ValueError(f"refusing to replace broad {label} directory {resolved}")
    if path.exists():
        if path.is_symlink() or not path.is_dir():
            raise ValueError(f"{label} staging path must be a real directory: {path}")
        shutil.rmtree(path)
    path.mkdir(parents=True)


def hash_filename(logical_name: str, digest: str) -> str:
    prefix = digest[:16]
    if "." in logical_name:
        stem, extension = logical_name.split(".", 1)
        return f"{stem}-{prefix}.{extension}"
    return f"{logical_name}-{prefix}"


def configured_evidence_artifacts(config_root: Path) -> dict[str, str]:
    """Map manifest evidence kinds to config-owned runtime filenames."""
    gate_config = config_root / "gate.toml"
    try:
        document = tomllib.loads(gate_config.read_text(encoding="utf-8"))
        configured = document["assets"]["evidence_artifacts"]
    except (OSError, tomllib.TOMLDecodeError, KeyError, TypeError) as error:
        raise ValueError(f"read configured asset evidence from {gate_config}: {error}") from error
    if not isinstance(configured, list) or not configured:
        raise ValueError("assets.evidence_artifacts must be a non-empty list")
    by_kind: dict[str, str] = {}
    for value in configured:
        logical_name = safe_component(value, "configured evidence artifact")
        kind = logical_name.split(".", 1)[0].replace("-", "_")
        if kind in by_kind:
            raise ValueError(f"configured evidence artifacts repeat manifest kind {kind}")
        by_kind[kind] = logical_name
    return by_kind


def local_file(url: object, label: str) -> Path:
    if not isinstance(url, str):
        raise ValueError(f"{label} lacks a staged URL")
    parsed = urlparse(url)
    if parsed.scheme != "file":
        raise ValueError(f"{label} was not resolved to a local immutable input")
    return Path(unquote(parsed.path))


def _stage_file(arch_dir: Path, logical_name: str, record: dict[str, Any], label: str) -> None:
    """Stage one verified input under its content-addressed and logical names."""
    digest = record.get("digest", {}).get("blake3")
    if not isinstance(digest, str):
        raise ValueError(f"{label} lacks BLAKE3")
    source = local_file(record.get("url"), label)
    shutil.copy2(source, arch_dir / hash_filename(logical_name, digest))
    shutil.copy2(source, arch_dir / logical_name)


def _active_runtime_architecture(manifest: dict[str, Any], arch: str) -> dict[str, Any]:
    runtime = manifest.get("runtime")
    if not isinstance(runtime, dict):
        raise ValueError("release manifest contains no runtime")
    if runtime.get("status") == "revoked":
        raise ValueError("release manifest runtime is revoked")
    source_commit = runtime.get("source_commit")
    if source_commit is not None and (
        not isinstance(source_commit, str)
        or len(source_commit) != 40
        or any(char not in "0123456789abcdef" for char in source_commit)
    ):
        raise ValueError("release runtime has malformed source_commit")
    matches = [
        candidate
        for candidate in runtime.get("architectures", [])
        if isinstance(candidate, dict) and candidate.get("architecture") == arch
    ]
    if len(matches) != 1:
        raise ValueError(f"release runtime must have exactly one {arch} architecture")
    return matches[0]


def _stage_runtime_architecture(
    architecture: dict[str, Any],
    *,
    arch: str,
    arch_dir: Path,
    evidence_artifacts: dict[str, str],
) -> None:
    """Stage boot images and the complete config-owned evidence closure."""
    images = architecture.get("images")
    if not isinstance(images, list):
        raise ValueError(f"release runtime/{arch} images are malformed")
    staged_images: set[str] = set()
    for index, value in enumerate(images):
        if not isinstance(value, dict):
            raise ValueError(f"release runtime/{arch} image[{index}] is malformed")
        record = cast(dict[str, Any], value)
        if record.get("status") == "revoked" or record.get("kind") not in IMAGE_NAMES:
            continue
        kind = cast(str, record["kind"])
        if kind in staged_images:
            raise ValueError(f"release runtime/{arch} repeats {kind} image")
        staged_images.add(kind)
        logical_name = safe_component(
            record.get("name") or IMAGE_NAMES[kind], f"runtime/{arch} {kind} image name"
        )
        _stage_file(arch_dir, logical_name, record, f"release runtime/{arch} {kind}")
    missing_images = set(IMAGE_NAMES) - staged_images
    if missing_images:
        raise ValueError(f"release runtime/{arch} lacks images: {sorted(missing_images)}")

    evidence = architecture.get("evidence")
    if not isinstance(evidence, list):
        raise ValueError(f"release runtime/{arch} evidence is malformed")
    staged_evidence: set[str] = set()
    for index, value in enumerate(evidence):
        if not isinstance(value, dict):
            raise ValueError(f"release runtime/{arch} evidence[{index}] is malformed")
        record = cast(dict[str, Any], value)
        if record.get("status") == "revoked":
            continue
        kind = record.get("kind")
        logical_name = evidence_artifacts.get(kind) if isinstance(kind, str) else None
        if logical_name is None:
            continue
        if logical_name in staged_evidence:
            raise ValueError(f"release runtime/{arch} repeats {logical_name}")
        staged_evidence.add(logical_name)
        _stage_file(arch_dir, logical_name, record, f"release runtime/{arch} {logical_name}")
    missing_evidence = set(evidence_artifacts.values()) - staged_evidence
    if missing_evidence:
        raise ValueError(
            f"release runtime/{arch} lacks configured evidence: {sorted(missing_evidence)}"
        )


def stage_runtime(input_dir: Path, assets_dir: Path) -> Path:
    """Stage the host architecture's verified runtime and its local manifest."""
    report, manifest = _load(input_dir)
    if report.get("kind") != "runtime":
        raise ValueError("runtime staging requires runtime release inputs")
    arch = _host_arch()
    selected_arch = report.get("architecture")
    if selected_arch is not None and selected_arch != arch:
        raise ValueError(f"runtime release inputs select {selected_arch}, not host {arch}")
    manifest_url = report.get("manifest_url")
    if not isinstance(manifest_url, str):
        raise ValueError("release input report lacks its manifest URL")
    _rewrite_urls(manifest, _local_url_map(input_dir, report), manifest_url)
    architecture = _active_runtime_architecture(manifest, arch)
    evidence_artifacts = configured_evidence_artifacts(ROOT / "config")
    _reset_staging_directory(assets_dir, "runtime asset")
    manifest_path = assets_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    arch_dir = assets_dir / arch
    arch_dir.mkdir(parents=True)
    _stage_runtime_architecture(
        architecture, arch=arch, arch_dir=arch_dir, evidence_artifacts=evidence_artifacts
    )
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
    host_arch = _host_arch()
    package_architectures = {"arm64"} if host_arch == "arm64" else {"amd64", "x86_64"}
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
        and package.get("architecture") in package_architectures
        and urljoin(str(report["manifest_url"]), str(package.get("url"))) in url_to_path
    ]
    if len(candidates) != 1:
        expected = "/".join(sorted(package_architectures))
        raise ValueError(f"expected one current Linux {expected} package, found {len(candidates)}")
    package = candidates[0]
    package_url = urljoin(str(report["manifest_url"]), str(package["url"]))
    return package, url_to_path[package_url]


def select_host_package_path(input_dir: Path) -> Path | None:
    # None when the cohort declares no package: a channel's first release. One
    # that declares packages and cannot produce one is still an error -- that is
    # a deleted public release, not a channel that never had one.
    if not _load(input_dir)[1].get("packages"):
        return None
    return _select_host_package(input_dir)[1]


def functional_binary_cohort_readiness(input_dir: Path) -> dict[str, Any]:
    """Report whether the pulled package can run the complete release modules."""
    report, _ = _load(input_dir)
    if report.get("allow_empty_packages") and not report.get("artifacts"):
        # A channel being cold-started has no published package to pair with,
        # so there is nothing to run the functional modules against. That is an
        # answer, not an error: the runtime stages deferred and the binary
        # release that follows publishes this channel's packages and activates.
        return {
            "ready": False,
            "missing": sorted(REQUIRED_LINUX_RELEASE_BINARIES),
            "unexpected": [],
        }
    package, _ = _select_host_package(input_dir)
    inventory = package.get("binaries")
    if not isinstance(inventory, list):
        raise ValueError("selected host package has no binary inventory")
    names: set[str] = set()
    for index, record in enumerate(inventory):
        if not isinstance(record, dict):
            raise ValueError(f"package binary[{index}] inventory row is malformed")
        record = cast(dict[str, Any], record)
        if record.get("status") == "revoked":
            continue
        names.add(safe_component(record.get("name"), f"package binary[{index}] inventory name"))
    missing = sorted(REQUIRED_LINUX_RELEASE_BINARIES - names)
    unexpected = sorted(names - REQUIRED_LINUX_RELEASE_BINARIES)
    return {
        "ready": not missing and not unexpected,
        "missing": missing,
        "unexpected": unexpected,
    }


def _extract_binaries(package_path: Path, extract_dir: Path) -> None:
    """Extract only regular Capsem executables through the portable reader."""
    if extract_dir.exists():
        shutil.rmtree(extract_dir)
    extract_dir.mkdir(parents=True)
    payloads = deb_payload_files(
        package_path,
        select=lambda name: (
            name.startswith("/usr/bin/capsem") and "/" not in name.removeprefix("/usr/bin/")
        ),
    )
    for name, payload in payloads.items():
        target = extract_dir / safe_relative(name.removeprefix("/"), "package binary path")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)


def stage_package_binaries(input_dir: Path, binary_dir: Path) -> list[Path]:
    package, package_path = _select_host_package(input_dir)
    extract_dir = binary_dir.parent / "resolved-package"
    _extract_binaries(package_path, extract_dir)
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
    _extract_binaries(package_path, extract_dir)
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
    parser.add_argument("--assets-dir", type=Path, default=Path("cache/target/assets"))
    parser.add_argument("--binary-dir", type=Path, default=Path("cache/target/cargo/debug"))
    parser.add_argument("--print-package-path", action="store_true")
    parser.add_argument("--check-functional-cohort", action="store_true")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    try:
        if args.check_functional_cohort:
            if args.input_dir is None:
                raise ValueError("--check-functional-cohort requires --input-dir")
            readiness = functional_binary_cohort_readiness(args.input_dir)
            if args.github_output is not None:
                with args.github_output.open("a", encoding="utf-8") as output:
                    output.write(f"functional-ready={str(readiness['ready']).lower()}\n")
            print(json.dumps(readiness, indent=2, sort_keys=True))
            return 0
        if args.print_package_path:
            if args.input_dir is None:
                raise ValueError("--print-package-path requires --input-dir")
            print(select_host_package_path(args.input_dir) or "")
            return 0
        if args.package_file is not None:
            result = stage_candidate_package(args.package_file, args.binary_dir)
        else:
            report, _ = _load(args.input_dir)
            if report.get("kind") == "runtime":
                result = [stage_runtime(args.input_dir, args.assets_dir)]
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
