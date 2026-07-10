#!/usr/bin/env python3
"""Validate the public Capsem binary release after channel deployment."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import platform
import shutil
import shlex
import subprocess
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
import zlib
from dataclasses import dataclass
from io import BytesIO
from pathlib import Path
from typing import Any


PROJECT_ROOT = Path(__file__).resolve().parents[1]
SBOM_GENERATOR = PROJECT_ROOT / "scripts" / "generate-host-binary-sbom.py"


@dataclass(frozen=True)
class RequiredPackage:
    platform: str
    architecture: str
    kind: str

    @classmethod
    def parse(cls, value: str) -> "RequiredPackage":
        parts = value.split(":")
        if len(parts) != 3 or not all(parts):
            raise argparse.ArgumentTypeError(
                "--required-package must use platform:architecture:kind"
            )
        return cls(parts[0], parts[1], parts[2])

    def label(self) -> str:
        return f"{self.platform}/{self.architecture}/{self.kind}"


DEFAULT_REQUIRED_PACKAGES = (
    RequiredPackage("macos", "arm64", "macos_pkg"),
    RequiredPackage("linux", "x86_64", "debian_package"),
    RequiredPackage("linux", "arm64", "debian_package"),
)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Fetch a public release-channel manifest, download its current "
            "host packages, and prove the per-binary manifest hashes match "
            "the executable files inside those packages."
        )
    )
    parser.add_argument("--channel", default="stable")
    parser.add_argument("--release-base-url", default="https://release.capsem.org")
    parser.add_argument("--manifest-url")
    parser.add_argument("--install-script-url", default="https://capsem.org/install.sh")
    parser.add_argument("--site-url")
    parser.add_argument("--package-dir", type=Path)
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument(
        "--required-package",
        action="append",
        type=RequiredPackage.parse,
        dest="required_packages",
        help=(
            "Required current package as platform:architecture:kind. Defaults "
            "to macOS arm64 .pkg plus Linux amd64/arm64 .deb."
        ),
    )
    parser.add_argument(
        "--docker-linux-install",
        action="store_true",
        help="Run curl -fsSL install.sh | sh in a clean Ubuntu Docker container.",
    )
    parser.add_argument(
        "--docker-channel-switch",
        action="store_true",
        help="After Docker install, switch assets stable -> nightly -> stable by manifest URL.",
    )
    parser.add_argument(
        "--docker-upgrade",
        action="store_true",
        help="After Docker install, run the binary updater against the nightly manifest URL.",
    )
    parser.add_argument("--nightly-manifest-url")
    parser.add_argument("--docker-image", default="ubuntu:24.04")
    args = parser.parse_args()

    manifest_url = args.manifest_url or (
        f"{args.release_base_url.rstrip('/')}/assets/{args.channel}/manifest.json"
    )
    nightly_manifest_url = args.nightly_manifest_url or (
        f"{args.release_base_url.rstrip('/')}/assets/nightly/manifest.json"
    )
    required = tuple(args.required_packages or DEFAULT_REQUIRED_PACKAGES)
    failures: list[str] = []

    try:
        install_script = fetch_text(args.install_script_url)
        failures.extend(
            check_install_script_defaults(
                install_script,
                channel=args.channel,
                release_base_url=args.release_base_url,
            )
        )
        if args.site_url:
            failures.extend(
                check_public_site_download_links(
                    fetch_text(args.site_url),
                    site_url=args.site_url,
                    channel=args.channel,
                    release_base_url=args.release_base_url,
                )
            )

        manifest = json.loads(fetch_bytes(manifest_url).decode("utf-8"))
        packages = current_packages_by_requirement(manifest, required, failures)

        with managed_work_dir(args.work_dir) as work_dir:
            validated_packages = 0
            validated_binaries = 0
            for requirement, package in packages.items():
                package_path = materialize_package(package, args.package_dir, work_dir, failures)
                if package_path is None:
                    continue
                failures.extend(check_package_url(package))
                failures.extend(check_package_digest(package, package_path))
                failures.extend(
                    check_package_manifest_origin(
                        package,
                        package_path,
                        expected_manifest_url=manifest_url,
                    )
                )
                binary_count, binary_failures = check_package_binaries(package, package_path, work_dir)
                failures.extend(binary_failures)
                validated_packages += 1
                validated_binaries += binary_count

            if args.docker_linux_install:
                run_docker_install_smoke(
                    channel=args.channel,
                    release_base_url=args.release_base_url,
                    install_script_url=args.install_script_url,
                    stable_manifest_url=manifest_url,
                    nightly_manifest_url=nightly_manifest_url,
                    channel_switch=args.docker_channel_switch,
                    upgrade=args.docker_upgrade,
                    docker_image=args.docker_image,
                )

        if failures:
            for failure in failures:
                print(f"error: {failure}", file=sys.stderr)
            return 1

        print(
            f"validated {validated_packages} package"
            f"{'' if validated_packages == 1 else 's'} and {validated_binaries} "
            f"packaged binaries from {manifest_url}"
        )
        return 0
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


def check_install_script_defaults(
    script: str,
    *,
    channel: str,
    release_base_url: str,
) -> list[str]:
    failures: list[str] = []
    if f'CAPSEM_CHANNEL="${{CAPSEM_CHANNEL:-{channel}}}"' not in script:
        failures.append(f"install.sh does not default CAPSEM_CHANNEL to {channel}")
    if (
        f'CAPSEM_RELEASE_BASE_URL="${{CAPSEM_RELEASE_BASE_URL:-{release_base_url}}}"'
        not in script
    ):
        failures.append(
            f"install.sh does not default CAPSEM_RELEASE_BASE_URL to {release_base_url}"
        )
    if "/assets/${CAPSEM_CHANNEL}/manifest.json" not in script:
        failures.append("install.sh does not resolve packages through the release channel")
    if "releases/latest" in script or "api.github.com/repos" in script:
        failures.append("install.sh still depends on GitHub latest release metadata")
    if "releases/tag/assets-" in script or "assets-v" in script:
        failures.append("install.sh contains an asset-release tag URL")
    return failures


def check_public_site_download_links(
    html: str,
    *,
    site_url: str,
    channel: str,
    release_base_url: str,
) -> list[str]:
    failures: list[str] = []
    if "releases/tag/assets-" in html or "assets-v" in html:
        failures.append(f"{site_url} contains an asset-release tag download URL")
    channel_manifest = (
        f"{release_base_url.rstrip('/')}/assets/{channel}/manifest.json"
    )
    has_install_entrypoint = "https://capsem.org/install.sh" in html or "install.sh" in html
    has_channel_entrypoint = channel_manifest in html or f"/assets/{channel}/manifest.json" in html
    if not has_install_entrypoint and not has_channel_entrypoint:
        failures.append(
            f"{site_url} does not expose the {channel} release-channel install entrypoint"
        )
    return failures


def current_packages_by_requirement(
    manifest: dict[str, Any],
    required: tuple[RequiredPackage, ...],
    failures: list[str],
) -> dict[RequiredPackage, dict[str, Any]]:
    raw_packages = manifest.get("packages")
    if not isinstance(raw_packages, list):
        failures.append("manifest packages missing or not a list")
        return {}

    packages: dict[RequiredPackage, dict[str, Any]] = {}
    for requirement in required:
        matches = [
            item
            for item in raw_packages
            if isinstance(item, dict)
            and item.get("status") == "current"
            and item.get("platform") == requirement.platform
            and item.get("architecture") == requirement.architecture
            and item.get("kind") == requirement.kind
        ]
        if len(matches) != 1:
            failures.append(
                f"manifest must contain exactly one current {requirement.label()} package"
            )
            continue
        packages[requirement] = matches[0]

    versions = {
        package.get("version")
        for package in packages.values()
        if isinstance(package.get("version"), str)
    }
    if len(versions) > 1:
        failures.append(f"current package versions disagree: {', '.join(sorted(versions))}")
    return packages


def materialize_package(
    package: dict[str, Any],
    package_dir: Path | None,
    work_dir: Path,
    failures: list[str],
) -> Path | None:
    name = package.get("name")
    if not isinstance(name, str) or not name:
        failures.append("package name missing")
        return None
    if package_dir is not None:
        path = package_dir / name
        if not path.is_file():
            failures.append(f"{name} missing from package directory {package_dir}")
            return None
        return path

    url = package.get("url")
    if not isinstance(url, str) or not url:
        failures.append(f"package {name} URL missing")
        return None
    path = work_dir / "packages" / name
    path.parent.mkdir(parents=True, exist_ok=True)
    print(f"download {url}")
    path.write_bytes(fetch_bytes(url))
    return path


def check_package_url(package: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    name = package.get("name")
    version = package.get("version")
    url = package.get("url")
    if not isinstance(name, str) or not isinstance(version, str) or not isinstance(url, str):
        return ["package name/version/url missing"]

    parsed = urllib.parse.urlparse(url)
    if parsed.scheme in {"http", "https"}:
        expected_path = f"/google/capsem/releases/download/v{version}/{name}"
        if parsed.netloc != "github.com" or parsed.path != expected_path:
            failures.append(
                f"package {name} must point at GitHub release download v{version}, got {url}"
            )
    if "/releases/tag/" in parsed.path or "assets-v" in parsed.path:
        failures.append(f"package {name} URL points at an asset tag instead of a binary release")
    return failures


def check_package_digest(package: dict[str, Any], package_path: Path) -> list[str]:
    digest = package.get("digest")
    if not isinstance(digest, dict):
        return [f"package {package.get('name', '<unknown>')} digest missing"]
    expected = digest.get("sha256")
    if not isinstance(expected, str) or not expected:
        return [f"package {package.get('name', '<unknown>')} SHA-256 missing"]
    actual = hashlib.sha256(package_path.read_bytes()).hexdigest()
    if actual != expected.lower():
        return [f"package {package_path.name} SHA-256 mismatch"]
    return []


def check_package_manifest_origin(
    package: dict[str, Any],
    package_path: Path,
    *,
    expected_manifest_url: str,
) -> list[str]:
    kind = package.get("kind")
    if kind == "debian_package":
        origin_path = "/usr/share/capsem/assets/manifest-origin.json"
        frozen_manifest_path = "/usr/share/capsem/assets/manifest.json"
    elif kind == "macos_pkg":
        origin_path = "/usr/local/share/capsem/assets/manifest-origin.json"
        frozen_manifest_path = "/usr/local/share/capsem/assets/manifest.json"
    else:
        return []

    try:
        payload = package_payload_files(package_path)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        return [f"could not inspect package metadata in {package_path.name}: {error}"]

    failures: list[str] = []
    if frozen_manifest_path in payload:
        failures.append(f"{package_path.name} freezes {frozen_manifest_path}")
    raw_origin = payload.get(origin_path)
    if raw_origin is None:
        failures.append(f"{package_path.name} missing {origin_path}")
        return failures
    try:
        origin = json.loads(raw_origin.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        return failures + [f"{package_path.name} has invalid manifest-origin.json: {error}"]
    if origin.get("schema") != "capsem.manifest_origin.v1":
        failures.append(f"{package_path.name} manifest-origin schema invalid")
    if origin.get("origin") != "package":
        failures.append(f"{package_path.name} manifest-origin origin must be package")
    if origin.get("source") != expected_manifest_url:
        failures.append(
            f"{package_path.name} manifest-origin source {origin.get('source')!r} "
            f"does not match {expected_manifest_url}"
        )
    if origin.get("package_version") != package.get("version"):
        failures.append(
            f"{package_path.name} manifest-origin package_version "
            f"{origin.get('package_version')!r} does not match {package.get('version')}"
        )
    if "snapshot_sha256" in origin:
        failures.append(f"{package_path.name} manifest-origin still records snapshot_sha256")
    return failures


def package_payload_files(package_path: Path) -> dict[str, bytes]:
    if package_path.name.endswith(".deb"):
        return deb_payload_files(package_path)
    if package_path.name.endswith(".pkg"):
        return pkg_payload_files(package_path)
    return {}


def deb_payload_files(package_path: Path) -> dict[str, bytes]:
    contents = package_path.read_bytes()
    offset = 8
    if not contents.startswith(b"!<arch>\n"):
        raise ValueError("invalid ar header")
    data_member: bytes | None = None
    data_member_name = "data.tar"
    while offset + 60 <= len(contents):
        header = contents[offset : offset + 60]
        name = header[:16].decode("ascii", errors="replace").strip().rstrip("/")
        size = int(header[48:58].decode("ascii").strip())
        data_start = offset + 60
        data_end = data_start + size
        data = contents[data_start:data_end]
        if name.startswith("data.tar"):
            data_member = data
            data_member_name = name
            break
        offset = data_end + (size % 2)
    if data_member is None:
        raise ValueError("missing data.tar member")
    return tar_payload_files(data_member, data_member_name)


def tar_payload_files(payload: bytes, member_name: str) -> dict[str, bytes]:
    try:
        with tarfile.open(fileobj=BytesIO(payload), mode="r:*") as archive:
            rows: dict[str, bytes] = {}
            for member in archive.getmembers():
                if not member.isfile():
                    continue
                handle = archive.extractfile(member)
                if handle is None:
                    continue
                rows[normalize_payload_path(member.name)] = handle.read()
            return rows
    except tarfile.TarError:
        with tempfile.TemporaryDirectory() as raw_tmp:
            raw = Path(raw_tmp)
            archive_path = raw / member_name
            payload_dir = raw / "payload"
            archive_path.write_bytes(payload)
            payload_dir.mkdir()
            subprocess.run(
                ["tar", "xf", str(archive_path.resolve()), "-C", str(payload_dir)],
                check=True,
                capture_output=True,
            )
            return {
                normalize_payload_path(path.relative_to(payload_dir).as_posix()): path.read_bytes()
                for path in payload_dir.rglob("*")
                if path.is_file()
            }


def pkg_payload_files(package_path: Path) -> dict[str, bytes]:
    if shutil.which("pkgutil") is not None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            expanded = Path(raw_tmp) / "expanded"
            subprocess.run(
                ["pkgutil", "--expand-full", str(package_path.resolve()), str(expanded)],
                check=True,
                capture_output=True,
            )
            rows: dict[str, bytes] = {}
            for payload in [path for path in expanded.rglob("Payload") if path.is_dir()]:
                for path in payload.rglob("*"):
                    if path.is_file():
                        rows[normalize_payload_path(path.relative_to(payload).as_posix())] = (
                            path.read_bytes()
                        )
            return rows
    return xar_pkg_payload_files(package_path)


def xar_pkg_payload_files(package_path: Path) -> dict[str, bytes]:
    contents = package_path.read_bytes()
    if len(contents) < 28 or contents[:4] != b"xar!":
        raise ValueError("not a xar .pkg archive")
    header_size = int.from_bytes(contents[4:6], "big")
    compressed_toc_size = int.from_bytes(contents[8:16], "big")
    toc_end = header_size + compressed_toc_size
    if header_size < 28 or toc_end > len(contents):
        raise ValueError("invalid xar header")
    toc = zlib.decompress(contents[header_size:toc_end]).decode("utf-8")
    rows: dict[str, bytes] = {}
    search_from = 0
    while True:
        name_index = toc.find("<name>Payload</name>", search_from)
        if name_index < 0:
            break
        block_start = toc.rfind("<file", 0, name_index)
        block_end = toc.find("</file>", name_index)
        if block_start < 0 or block_end < 0:
            raise ValueError("malformed Payload metadata")
        block = toc[block_start : block_end + len("</file>")]
        offset = int(xml_tag(block, "offset"))
        length = int(xml_tag(block, "length"))
        payload = contents[toc_end + offset : toc_end + offset + length]
        if len(payload) != length:
            raise ValueError("truncated Payload")
        if "application/x-gzip" in block or payload.startswith(b"\x1f\x8b"):
            payload = gzip.decompress(payload)
        rows.update(cpio_payload_files(payload))
        search_from = block_end + len("</file>")
    return rows


def xml_tag(block: str, tag: str) -> str:
    start = block.find(f"<{tag}>")
    end = block.find(f"</{tag}>", start)
    if start < 0 or end < 0:
        raise ValueError(f"xar Payload metadata missing {tag}")
    return block[start + len(tag) + 2 : end].strip()


def cpio_payload_files(payload: bytes) -> dict[str, bytes]:
    if payload.startswith(b"070707"):
        return odc_cpio_payload_files(payload)
    rows: dict[str, bytes] = {}
    offset = 0
    while offset < len(payload):
        header = payload[offset : offset + 110]
        if len(header) < 110:
            raise ValueError("newc cpio header truncated")
        if header[:6] not in {b"070701", b"070702"}:
            raise ValueError("newc cpio header magic mismatch")
        mode = int(header[14:22], 16)
        file_size = int(header[54:62], 16)
        name_size = int(header[94:102], 16)
        name_start = offset + 110
        name_end = name_start + name_size
        name = payload[name_start : name_end - 1].decode("utf-8")
        data_start = align4(name_end)
        data_end = data_start + file_size
        if name == "TRAILER!!!":
            break
        if mode & 0o170000 == 0o100000:
            rows[normalize_payload_path(name)] = payload[data_start:data_end]
        offset = align4(data_end)
    return rows


def odc_cpio_payload_files(payload: bytes) -> dict[str, bytes]:
    rows: dict[str, bytes] = {}
    offset = 0
    while offset < len(payload):
        header = payload[offset : offset + 76]
        if len(header) < 76:
            raise ValueError("odc cpio header truncated")
        if header[:6] != b"070707":
            raise ValueError("odc cpio header magic mismatch")
        mode = int(header[18:24], 8)
        name_size = int(header[59:65], 8)
        file_size = int(header[65:76], 8)
        name_start = offset + 76
        name_end = name_start + name_size
        name = payload[name_start : name_end - 1].decode("utf-8")
        data_start = name_end
        data_end = data_start + file_size
        if name == "TRAILER!!!":
            break
        if mode & 0o170000 == 0o100000:
            rows[normalize_payload_path(name)] = payload[data_start:data_end]
        offset = data_end
    return rows


def align4(value: int) -> int:
    return (value + 3) & ~3


def normalize_payload_path(path: str) -> str:
    return "/" + path.removeprefix("./").lstrip("/")


def check_package_binaries(
    package: dict[str, Any],
    package_path: Path,
    work_dir: Path,
) -> tuple[int, list[str]]:
    binaries = package.get("binaries")
    if not isinstance(binaries, list) or not binaries:
        return 0, [f"package {package_path.name} binaries missing or empty"]

    sbom_path = work_dir / "sbom" / f"{package_path.name}.spdx.json"
    sbom_path.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        [
            sys.executable,
            str(SBOM_GENERATOR),
            "--output",
            str(sbom_path),
            str(package_path),
        ],
        cwd=PROJECT_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        return 0, [f"could not inspect {package_path.name}: {detail}"]
    sbom = json.loads(sbom_path.read_text(encoding="utf-8"))
    entries = sbom_file_hashes(sbom)

    failures: list[str] = []
    count = 0
    payload: dict[str, bytes] | None = None
    for binary in binaries:
        if not isinstance(binary, dict):
            failures.append(f"package {package_path.name} contains non-object binary entry")
            continue
        installed_path = binary.get("installed_path")
        digest = binary.get("digest")
        expected_sha256 = digest.get("sha256") if isinstance(digest, dict) else None
        if not isinstance(installed_path, str) or not installed_path.startswith("/"):
            failures.append(f"package {package_path.name} binary installed_path invalid")
            continue
        if not isinstance(expected_sha256, str) or not expected_sha256:
            failures.append(f"binary {installed_path} SHA-256 missing")
            continue
        if (installed_path, expected_sha256.lower()) not in entries:
            failures.append(
                f"binary {installed_path} SHA-256 not found inside {package_path.name}"
            )
            continue
        if should_execute_packaged_binary(package, installed_path):
            if payload is None:
                try:
                    payload = package_payload_files(package_path)
                except (OSError, ValueError, subprocess.CalledProcessError) as error:
                    failures.append(f"could not extract {package_path.name}: {error}")
                    payload = {}
            failures.extend(
                check_packaged_binary_version(
                    package,
                    binary,
                    installed_path,
                    payload,
                    work_dir,
                    package_path.name,
                )
            )
        count += 1
    return count, failures


def should_execute_packaged_binary(package: dict[str, Any], installed_path: str) -> bool:
    if os.environ.get("CAPSEM_SKIP_PACKAGE_EXECUTION") == "1":
        return False
    if package.get("platform") != "linux" or package.get("architecture") != "x86_64":
        return False
    if package.get("kind") != "debian_package":
        return False
    if Path(installed_path).name in {"capsem-app", "capsem-tray"}:
        return False
    return platform.system() == "Linux" and platform.machine().lower() in {"x86_64", "amd64"}


def check_packaged_binary_version(
    package: dict[str, Any],
    binary: dict[str, Any],
    installed_path: str,
    payload: dict[str, bytes],
    work_dir: Path,
    package_name: str,
) -> list[str]:
    contents = payload.get(installed_path)
    if contents is None:
        return [f"binary {installed_path} missing from {package_name} payload"]
    expected_version = binary.get("version") or package.get("version")
    if not isinstance(expected_version, str) or not expected_version:
        return [f"binary {installed_path} version missing"]

    executable = work_dir / "exec" / package_name / installed_path.removeprefix("/")
    executable.parent.mkdir(parents=True, exist_ok=True)
    executable.write_bytes(contents)
    executable.chmod(0o755)
    command = [str(executable), "version"] if executable.name == "capsem" else [str(executable), "--version"]
    result = subprocess.run(command, capture_output=True, text=True, timeout=10)
    output = f"{result.stdout}\n{result.stderr}".strip()
    if result.returncode != 0:
        return [
            f"binary {installed_path} version command failed with {result.returncode}: {output}"
        ]
    if expected_version not in output:
        return [
            f"binary {installed_path} version output does not contain {expected_version}: {output}"
        ]
    return []


def sbom_file_hashes(sbom: dict[str, Any]) -> set[tuple[str, str]]:
    rows: set[tuple[str, str]] = set()
    for file_entry in sbom.get("files", []):
        if not isinstance(file_entry, dict):
            continue
        file_name = file_entry.get("fileName")
        if not isinstance(file_name, str):
            continue
        for checksum in file_entry.get("checksums", []):
            if (
                isinstance(checksum, dict)
                and checksum.get("algorithm") == "SHA256"
                and isinstance(checksum.get("checksumValue"), str)
            ):
                rows.add((file_name, checksum["checksumValue"].lower()))
    return rows


def run_docker_install_smoke(
    *,
    channel: str,
    release_base_url: str,
    install_script_url: str,
    stable_manifest_url: str,
    nightly_manifest_url: str,
    channel_switch: bool,
    upgrade: bool,
    docker_image: str,
) -> None:
    if shutil.which("docker") is None:
        raise OSError("docker is required for --docker-linux-install")
    install_pipeline = (
        f"curl -fsSL {shlex.quote(install_script_url)} | "
        f"CAPSEM_CHANNEL={shlex.quote(channel)} "
        f"CAPSEM_RELEASE_BASE_URL={shlex.quote(release_base_url)} sh"
    )
    helper_checks = " ".join(
        shlex.quote(binary)
        for binary in (
            "capsem",
            "capsem-admin",
            "capsem-gateway",
            "capsem-mcp",
            "capsem-mcp-aggregator",
            "capsem-mcp-builtin",
            "capsem-process",
            "capsem-service",
            "capsem-tray",
            "capsem-tui",
        )
    )
    container_script = f"""
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl sudo
useradd -m -s /bin/bash capsemtest
printf '%s\\n' 'capsemtest ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/capsemtest
chmod 0440 /etc/sudoers.d/capsemtest
su capsemtest -c {shlex.quote(install_pipeline)}
su capsemtest -c 'test -x "$HOME/.capsem/bin/capsem"'
su capsemtest -c '"$HOME/.capsem/bin/capsem" --version'
su capsemtest -c 'test -f "$HOME/.capsem/assets/manifest.json"'
su capsemtest -c 'grep -F {shlex.quote(stable_manifest_url)} "$HOME/.capsem/assets/manifest-origin.json"'
for bin in {helper_checks}; do
  su capsemtest -c "test -x \\"\\$HOME/.capsem/bin/$bin\\""
  su capsemtest -c "\\"\\$HOME/.capsem/bin/$bin\\" --version"
done
"""
    if channel_switch:
        container_script += f"""
su capsemtest -c 'CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" "$HOME/.capsem/bin/capsem" update --assets --manifest {shlex.quote(nightly_manifest_url)}'
su capsemtest -c 'grep -F {shlex.quote(nightly_manifest_url)} "$HOME/.capsem/assets/manifest-origin.json"'
su capsemtest -c 'CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" "$HOME/.capsem/bin/capsem" update --assets --manifest {shlex.quote(stable_manifest_url)}'
su capsemtest -c 'grep -F {shlex.quote(stable_manifest_url)} "$HOME/.capsem/assets/manifest-origin.json"'
"""
    if upgrade:
        container_script += f"""
su capsemtest -c 'CAPSEM_HOME="$HOME/.capsem" CAPSEM_RUN_DIR="$HOME/.capsem/run" CAPSEM_RELEASE_MANIFEST_URL={shlex.quote(nightly_manifest_url)} DEBIAN_FRONTEND=noninteractive "$HOME/.capsem/bin/capsem" update --yes'
"""
    subprocess.run(
        ["docker", "run", "--rm", "--pull=missing", docker_image, "bash", "-lc", container_script],
        check=True,
    )


def fetch_text(location: str) -> str:
    return fetch_bytes(location).decode("utf-8")


def fetch_bytes(location: str) -> bytes:
    parsed = urllib.parse.urlparse(location)
    if parsed.scheme in {"http", "https"}:
        request = urllib.request.Request(location, headers={"User-Agent": "capsem-release-gate"})
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.read()
    if parsed.scheme == "file":
        return Path(urllib.request.url2pathname(parsed.path)).read_bytes()
    return Path(location).read_bytes()


class managed_work_dir:
    def __init__(self, path: Path | None) -> None:
        self.path = path
        self._tmp: tempfile.TemporaryDirectory[str] | None = None

    def __enter__(self) -> Path:
        if self.path is not None:
            self.path.mkdir(parents=True, exist_ok=True)
            return self.path
        self._tmp = tempfile.TemporaryDirectory()
        return Path(self._tmp.name)

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        if self._tmp is not None:
            self._tmp.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
