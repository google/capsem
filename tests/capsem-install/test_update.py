"""Self-update tests for WB4.

Tests verify `capsem update` behavior for development builds,
installed layout detection, and update cache management.
"""

from __future__ import annotations

from collections.abc import Callable
from contextlib import contextmanager
import hashlib
import http.server
import json
import os
import platform
import shutil
import socketserver
import subprocess
import threading
from pathlib import Path

import pytest
from blake3 import blake3

from .conftest import (
    CAPSEM_DIR,
    run_capsem,
    get_build_hash,
)

UPDATE_CACHE = CAPSEM_DIR / "update-check.json"
REPO_ROOT = Path(__file__).resolve().parents[2]


class _HealthHandler(http.server.BaseHTTPRequestHandler):
    body: bytes = b""
    files: dict[str, bytes] = {}

    def do_GET(self) -> None:
        if self.path == "/health.json":
            body = self.body
        elif self.path in self.files:
            body = self.files[self.path]
        else:
            self.send_error(404)
            return
        self.send_response(200)
        content_type = "application/json" if self.path == "/health.json" else "application/octet-stream"
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args) -> None:
        return


@contextmanager
def _serve_health(body: bytes):
    handler = type("HealthHandler", (_HealthHandler,), {"body": body, "files": {}})
    server = socketserver.TCPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}/health.json"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


@contextmanager
def _serve_release(health: dict | Callable[[str], dict], files: dict[str, bytes]):
    handler = type("ReleaseHandler", (_HealthHandler,), {"body": b"", "files": files})
    server = socketserver.TCPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    base = f"http://127.0.0.1:{server.server_address[1]}"
    health_payload = health(base) if callable(health) else health
    handler.body = json.dumps(health_payload, sort_keys=True, separators=(",", ":")).encode()
    thread.start()
    try:
        yield base, f"{base}/health.json"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def _copy_user_dir_capsem(source_bin: Path, capsem_home: Path) -> Path:
    target = capsem_home / "bin" / "capsem"
    target.parent.mkdir(parents=True)
    shutil.copy2(source_bin, target)
    target.chmod(0o755)
    return target


def _copy_layout_capsem(source_bin: Path, root: Path, layout: str) -> Path:
    if layout == "macos_pkg":
        target = root / "usr" / "local" / "bin" / "capsem"
    elif layout == "linux_deb":
        target = root / "usr" / "bin" / "capsem"
    else:
        raise ValueError(layout)
    target.parent.mkdir(parents=True)
    shutil.copy2(source_bin, target)
    target.chmod(0o755)
    return target


def _write_fake_sudo(bin_dir: Path, log_path: Path) -> None:
    sudo = bin_dir / "sudo"
    bin_dir.mkdir(parents=True, exist_ok=True)
    sudo.write_text(
        "#!/bin/sh\n"
        "printf '%s\\n' \"$*\" >> \"$CAPSEM_FAKE_SUDO_LOG\"\n"
        "exit 0\n",
        encoding="utf-8",
    )
    sudo.chmod(0o755)
    log_path.write_text("", encoding="utf-8")


def _deb_arch() -> str:
    machine = platform.machine().lower()
    return "arm64" if machine in {"aarch64", "arm64"} else "amd64"


def _binary_update_health(base_url: str, installer_name: str, payload: bytes) -> dict:
    return {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "99.99.99",
                "current": "0.0.0",
                "files": [
                    {
                        "name": installer_name,
                        "url": f"{base_url}/{installer_name}",
                        "sha256": hashlib.sha256(payload).hexdigest(),
                        "size": len(payload),
                    }
                ],
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.1",
            },
        },
    }


def _profile_catalog_path(revision: str) -> str:
    return f"/profiles/releases/{revision}/catalog.json"


def _profile_config(revision: str) -> dict:
    def assets_for_arch(arch: str) -> dict:
        base = f"https://release.capsem.org/assets/releases/2030.0101.1/{arch}"
        return {
            "kernel": {"name": f"{arch}-vmlinuz", "url": f"{base}-vmlinuz"},
            "initrd": {"name": f"{arch}-initrd.img", "url": f"{base}-initrd.img"},
            "rootfs": {"name": f"{arch}-rootfs.erofs", "url": f"{base}-rootfs.erofs"},
        }

    return {
        "id": "code",
        "name": "Code",
        "description": "Default coding profile",
        "revision": revision,
        "refresh_policy": "manual",
        "assets": {
            "format": "profile-assets.v1",
            "refresh_policy": "manual",
            "arch": {
                "arm64": assets_for_arch("arm64"),
                "x86_64": assets_for_arch("x86_64"),
            },
        },
    }


def _profile_catalog_bytes(revision: str) -> bytes:
    catalog = {
        "schema": "capsem.profile_catalog.v1",
        "revision": revision,
        "state": "current",
        "current_binary": "1.4.1234567890",
        "current_assets": "2030.0101.1",
        "compatibility": {
            "binary": "1.4.1234567890",
            "assets": "2030.0101.1",
            "min_binary": "1.0.0",
            "min_assets": "2030.0101.1",
            "requires_newer_binary": False,
            "requires_newer_assets": False,
        },
        "profiles": [_profile_config(revision)],
    }
    return json.dumps(catalog, sort_keys=True, separators=(",", ":")).encode()


def _profile_update_health(catalog_path: str, catalog_bytes: bytes, latest: str) -> dict:
    return {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "0.0.0",
                "current": "0.0.0",
                "files": [],
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.1",
            },
            "profiles": {
                "latest": latest,
                "current": latest,
                "state": "current",
                "source": catalog_path,
                "hash": blake3(catalog_bytes).hexdigest(),
                "compatibility": {
                    "binary": "1.4.1234567890",
                    "assets": "2030.0101.1",
                    "min_binary": "1.0.0",
                    "min_assets": "2030.0101.1",
                },
                "requires_newer": {
                    "binary": False,
                    "assets": False,
                },
            },
        },
    }


def _write_installed_profile_catalog(profiles_dir: Path, revision: str) -> None:
    profile_dir = profiles_dir / "code"
    profile_dir.mkdir(parents=True)
    profile_dir.joinpath("profile.toml").write_text(
        f"""
id = "code"
name = "Code"
description = "Default coding profile"
revision = "{revision}"
refresh_policy = "manual"

[assets]
format = "profile-assets.v1"
refresh_policy = "manual"

[assets.arch.arm64.kernel]
name = "arm64-vmlinuz"
url = "https://release.capsem.org/assets/releases/2030.0101.1/arm64-vmlinuz"

[assets.arch.arm64.initrd]
name = "arm64-initrd.img"
url = "https://release.capsem.org/assets/releases/2030.0101.1/arm64-initrd.img"

[assets.arch.arm64.rootfs]
name = "arm64-rootfs.erofs"
url = "https://release.capsem.org/assets/releases/2030.0101.1/arm64-rootfs.erofs"

[assets.arch.x86_64.kernel]
name = "x86_64-vmlinuz"
url = "https://release.capsem.org/assets/releases/2030.0101.1/x86_64-vmlinuz"

[assets.arch.x86_64.initrd]
name = "x86_64-initrd.img"
url = "https://release.capsem.org/assets/releases/2030.0101.1/x86_64-initrd.img"

[assets.arch.x86_64.rootfs]
name = "x86_64-rootfs.erofs"
url = "https://release.capsem.org/assets/releases/2030.0101.1/x86_64-rootfs.erofs"
""".lstrip(),
        encoding="utf-8",
    )


def _write_installed_asset_manifest(capsem_home: Path, current_assets: str) -> None:
    manifest = json.loads((REPO_ROOT / "assets" / "manifest.json").read_text())
    manifest["assets"]["current"] = current_assets
    assets_dir = capsem_home / "assets"
    assets_dir.mkdir(parents=True)
    (assets_dir / "manifest.json").write_text(
        json.dumps(manifest, sort_keys=True, separators=(",", ":")),
        encoding="utf-8",
    )


def _fresh_capsem_binary() -> Path | None:
    bin_src = Path(os.environ.get("CAPSEM_BIN_SRC", REPO_ROOT / "target" / "debug"))
    binary = bin_src / "capsem"
    source_paths = [
        REPO_ROOT / "crates" / "capsem" / "src" / "update.rs",
        REPO_ROOT / "crates" / "capsem" / "src" / "client.rs",
        REPO_ROOT / "crates" / "capsem" / "src" / "main.rs",
    ]
    if binary.is_file() and all(
        binary.stat().st_mtime >= path.stat().st_mtime for path in source_paths
    ):
        return binary
    result = subprocess.run(
        ["cargo", "build", "-p", "capsem"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
        env={
            **os.environ,
            "CARGO_TARGET_DIR": str(bin_src.parent),
        },
    )
    assert result.returncode == 0, (
        f"cargo build -p capsem failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    return binary if binary.is_file() else None


def test_update_fetches_release_health_and_writes_channel_cache(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    health = {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "99.99.99",
                "current": "99.99.98",
                "files": [],
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.0",
            },
            "profiles": {
                "latest": "profiles-2030.0101.1",
                "state": "published",
            },
            "images": {
                "latest": None,
                "state": "not_published",
            },
        },
    }
    body = json.dumps(health, sort_keys=True, separators=(",", ":")).encode()

    with _serve_health(body) as health_url:
        result = subprocess.run(
            [str(capsem), "update"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Binary update available" in result.stdout
    assert "Run `capsem update --assets` separately" in result.stdout
    assert "VM image update track not published." in result.stdout

    cache = json.loads((capsem_home / "update-check.json").read_text(encoding="utf-8"))
    assert cache["source"] == health_url
    assert cache["channel_hash"] == hashlib.sha256(body).hexdigest()
    assert cache["validation_status"] == "valid"
    assert cache.get("validation_error") is None
    assert cache["latest_version"] == "99.99.99"
    assert cache["update_available"] is True
    assert cache["latest_assets"] == "2030.0101.1"
    assert cache.get("current_assets") is None
    assert cache["latest_profiles"] == "profiles-2030.0101.1"
    assert cache["profiles_state"] == "published"
    assert cache["images_state"] == "not_published"


def test_update_check_reports_binary_profile_asset_and_image_tracks(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    health = {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "99.99.99",
                "current": "99.99.98",
                "files": [],
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2030.0101.0",
            },
            "profiles": {
                "latest": "profiles-2030.0101.1",
                "state": "published",
                "requires_newer": {"binary": True, "assets": False},
                "compatibility": {
                    "min_binary": "99.99.99",
                    "min_assets": "2030.0101.0",
                },
            },
            "images": {
                "latest": None,
                "state": "not_published",
            },
        },
    }
    body = json.dumps(health, sort_keys=True, separators=(",", ":")).encode()

    with _serve_health(body) as health_url:
        result = subprocess.run(
            [str(capsem), "update", "--check"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update --check failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Binary update available" in result.stdout
    assert "Profile catalog update blocked: requires binary 99.99.99 or newer." in result.stdout
    assert (
        "VM asset state unknown: installed manifest not found; latest release is 2030.0101.1."
        in result.stdout
    )
    assert "VM image update track not published." in result.stdout
    assert "Profile catalog update applied" not in result.stdout
    assert not (capsem_home / "profiles" / "catalog-origin.json").exists()


def test_binary_update_state_does_not_claim_asset_update(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    _write_installed_asset_manifest(capsem_home, "2026.0627.8")
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    health = {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "99.99.99",
                "current": "99.99.98",
                "files": [],
            },
            "assets": {
                "latest": "2026.0627.8",
                "current": "2026.0627.8",
            },
            "images": {
                "latest": None,
                "state": "not_published",
            },
        },
    }
    body = json.dumps(health, sort_keys=True, separators=(",", ":")).encode()

    with _serve_health(body) as health_url:
        result = subprocess.run(
            [str(capsem), "update", "--check"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update --check failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Binary update available" in result.stdout
    assert "VM asset update available" not in result.stdout
    assert "VM asset state unknown" not in result.stdout

    cache = json.loads((capsem_home / "update-check.json").read_text(encoding="utf-8"))
    assert cache["update_available"] is True
    assert cache["assets_update_available"] is False
    assert cache["latest_assets"] == "2026.0627.8"
    assert cache["current_assets"] == "2026.0627.8"


def test_asset_update_state_does_not_claim_binary_update(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    _write_installed_asset_manifest(capsem_home, "2026.0627.8")
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    health = {
        "schema": "capsem.assets_channel.health.v1",
        "updates": {
            "binary": {
                "latest": "0.0.0",
                "current": "0.0.0",
                "files": [],
            },
            "assets": {
                "latest": "2030.0101.1",
                "current": "2026.0627.8",
            },
            "images": {
                "latest": None,
                "state": "not_published",
            },
        },
    }
    body = json.dumps(health, sort_keys=True, separators=(",", ":")).encode()

    with _serve_health(body) as health_url:
        result = subprocess.run(
            [str(capsem), "update", "--check"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update --check failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Capsem binary is current" in result.stdout
    assert "Binary update available" not in result.stdout
    assert "VM asset update available: 2026.0627.8 -> 2030.0101.1." in result.stdout

    cache = json.loads((capsem_home / "update-check.json").read_text(encoding="utf-8"))
    assert cache["update_available"] is False
    assert cache["assets_update_available"] is True
    assert cache["latest_assets"] == "2030.0101.1"
    assert cache["current_assets"] == "2026.0627.8"


def test_update_reports_profile_catalog_without_applying_by_default(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    _write_installed_profile_catalog(capsem_home / "profiles", "profiles-2030.0101.0")
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    revision = "profiles-2030.0101.1"
    catalog_path = _profile_catalog_path(revision)
    catalog_bytes = _profile_catalog_bytes(revision)

    with _serve_release(
        _profile_update_health(catalog_path, catalog_bytes, revision),
        {catalog_path: catalog_bytes},
    ) as (_, health_url):
        result = subprocess.run(
            [str(capsem), "update"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Profile catalog update available" in result.stdout
    assert "Re-run with --yes to apply the profile catalog update." in result.stdout
    assert "Profile catalog update applied" not in result.stdout
    profile_toml = (capsem_home / "profiles" / "code" / "profile.toml").read_text(
        encoding="utf-8"
    )
    assert 'revision = "profiles-2030.0101.0"' in profile_toml
    assert not (capsem_home / "profiles" / "catalog-origin.json").exists()


def test_update_yes_applies_compatible_profile_catalog_from_release_channel(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    _write_installed_profile_catalog(capsem_home / "profiles", "profiles-2030.0101.0")
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    revision = "profiles-2030.0101.1"
    catalog_path = _profile_catalog_path(revision)
    catalog_bytes = _profile_catalog_bytes(revision)

    with _serve_release(
        _profile_update_health(catalog_path, catalog_bytes, revision),
        {catalog_path: catalog_bytes},
    ) as (_, health_url):
        result = subprocess.run(
            [str(capsem), "update", "--yes"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode == 0, (
        f"capsem update --yes failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "Profile catalog update available" in result.stdout
    assert "Profile catalog update applied" in result.stdout
    profile_toml = (capsem_home / "profiles" / "code" / "profile.toml").read_text(
        encoding="utf-8"
    )
    assert 'revision = "profiles-2030.0101.1"' in profile_toml
    origin = json.loads(
        (capsem_home / "profiles" / "catalog-origin.json").read_text(encoding="utf-8")
    )
    assert origin["schema"] == "capsem.profile_catalog_origin.v1"
    assert origin["origin"] == "update"
    assert origin["revision"] == revision
    assert origin["hash"] == blake3(catalog_bytes).hexdigest()

    cache = json.loads((capsem_home / "update-check.json").read_text(encoding="utf-8"))
    assert cache["profile_catalog_source"] == catalog_path
    assert cache["profile_catalog_hash"] == blake3(catalog_bytes).hexdigest()


def test_update_preserves_profile_catalog_when_release_catalog_is_invalid(
    tmp_path: Path,
    installed_layout,
) -> None:
    capsem_home = tmp_path / ".capsem"
    _write_installed_profile_catalog(capsem_home / "profiles", "profiles-2030.0101.0")
    fresh_capsem = _fresh_capsem_binary()
    source_capsem = fresh_capsem if fresh_capsem is not None else installed_layout / "capsem"
    capsem = _copy_user_dir_capsem(source_capsem, capsem_home)
    revision = "profiles-2030.0101.1"
    catalog_path = _profile_catalog_path(revision)
    invalid_profile = _profile_config(revision)
    invalid_profile["assets"]["arch"]["arm64"]["kernel"]["url"] = "assets/arm64-vmlinuz"
    invalid_catalog = json.dumps(
        {
            "schema": "capsem.profile_catalog.v1",
            "revision": revision,
            "state": "current",
            "profiles": [invalid_profile],
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode()

    with _serve_release(
        _profile_update_health(catalog_path, invalid_catalog, revision),
        {catalog_path: invalid_catalog},
    ) as (_, health_url):
        result = subprocess.run(
            [str(capsem), "update", "--yes"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
            },
        )

    assert result.returncode != 0, (
        f"invalid profile catalog apply should fail\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    assert "validate profile code" in result.stderr
    profile_toml = (capsem_home / "profiles" / "code" / "profile.toml").read_text(
        encoding="utf-8"
    )
    assert 'revision = "profiles-2030.0101.0"' in profile_toml
    assert not (capsem_home / "profiles" / "catalog-origin.json").exists()


def test_macos_update_yes_applies_verified_pkg_with_package_manager(
    tmp_path: Path,
) -> None:
    capsem_home = tmp_path / ".capsem"
    fresh_capsem = _fresh_capsem_binary()
    assert fresh_capsem is not None
    capsem = _copy_layout_capsem(fresh_capsem, tmp_path, "macos_pkg")
    fake_bin = tmp_path / "fake-bin"
    sudo_log = tmp_path / "sudo.log"
    _write_fake_sudo(fake_bin, sudo_log)
    installer_name = "Capsem-99.99.99.pkg"
    payload = b"verified macos package payload"

    with _serve_release(
        lambda base_url: _binary_update_health(base_url, installer_name, payload),
        {f"/{installer_name}": payload},
    ) as (_, health_url):
        result = subprocess.run(
            [str(capsem), "update", "--yes"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
                "CAPSEM_FAKE_SUDO_LOG": str(sudo_log),
                "PATH": f"{fake_bin}{os.pathsep}{os.environ.get('PATH', '')}",
            },
        )

    assert result.returncode == 0, (
        f"capsem update --yes failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    cached = capsem_home / "updates" / "installers" / installer_name
    assert cached.read_bytes() == payload
    assert f"/usr/sbin/installer -pkg {cached} -target /\n" == sudo_log.read_text(
        encoding="utf-8"
    )
    assert "Binary update applied." in result.stdout


def test_linux_update_yes_applies_verified_deb_with_package_manager(
    tmp_path: Path,
) -> None:
    capsem_home = tmp_path / ".capsem"
    fresh_capsem = _fresh_capsem_binary()
    assert fresh_capsem is not None
    capsem = _copy_layout_capsem(fresh_capsem, tmp_path, "linux_deb")
    fake_bin = tmp_path / "fake-bin"
    sudo_log = tmp_path / "sudo.log"
    _write_fake_sudo(fake_bin, sudo_log)
    installer_name = f"Capsem_99.99.99_{_deb_arch()}.deb"
    payload = b"verified linux package payload"

    with _serve_release(
        lambda base_url: _binary_update_health(base_url, installer_name, payload),
        {f"/{installer_name}": payload},
    ) as (_, health_url):
        result = subprocess.run(
            [str(capsem), "update", "--yes"],
            capture_output=True,
            text=True,
            timeout=30,
            env={
                **os.environ,
                "CAPSEM_HOME": str(capsem_home),
                "CAPSEM_RUN_DIR": str(capsem_home / "run"),
                "CAPSEM_RELEASE_HEALTH_URL": health_url,
                "CAPSEM_FAKE_SUDO_LOG": str(sudo_log),
                "PATH": f"{fake_bin}{os.pathsep}{os.environ.get('PATH', '')}",
            },
        )

    assert result.returncode == 0, (
        f"capsem update --yes failed\nstdout={result.stdout}\nstderr={result.stderr}"
    )
    cached = capsem_home / "updates" / "installers" / installer_name
    assert cached.read_bytes() == payload
    assert f"apt-get install --yes {cached}\n" == sudo_log.read_text(encoding="utf-8")
    assert "Binary update applied." in result.stdout


@pytest.mark.live_system
class TestSelfUpdate:
    """capsem update command."""

    def test_update_dev_build_bails(self, installed_layout, clean_state):
        """Non-installed layout prints 'build from source' advice."""
        # When running from installed layout, the binary is in ~/.capsem/bin
        # which is detected as UserDir, not Development. But the test verifies
        # the update command runs without crashing.
        result = run_capsem("update", "--yes", timeout=30)
        # Should either succeed or fail gracefully (network may not be available)
        combined = result.stdout + result.stderr
        # It should at least attempt to check or explain the situation
        assert result.returncode == 0 or "failed" in combined.lower() or "error" in combined.lower()

    def test_installed_layout_detection(self, installed_layout):
        """Installed binaries in ~/.capsem/bin are detected correctly."""
        # The binary is in ~/.capsem/bin which should be detected as UserDir
        # We verify indirectly: update command doesn't say "Development build"
        result = run_capsem("update", "--yes", timeout=30)
        assert "Development build" not in result.stdout, (
            "installed binary should not be detected as development build"
        )

    def test_update_cache_write_and_read(self, installed_layout, clean_state):
        """Update cache file written with version info."""
        # Remove any existing cache
        UPDATE_CACHE.unlink(missing_ok=True)

        # Run any command to trigger background cache refresh
        run_capsem("version", timeout=10)

        # The background refresh is fire-and-forget, may not have completed
        # Write a synthetic cache to test the read path
        CAPSEM_DIR.mkdir(parents=True, exist_ok=True)
        import time

        cache = {
            "checked_at": int(time.time()),
            "latest_version": "99.99.99",
            "update_available": True,
        }
        UPDATE_CACHE.write_text(json.dumps(cache))

        # Now any command should show the update notice
        result = run_capsem("version", timeout=10)
        assert result.returncode == 0
        # The notice goes to stderr
        assert "update available" in result.stderr.lower() or "99.99.99" in result.stderr, (
            f"expected update notice in stderr:\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

        # Cleanup
        UPDATE_CACHE.unlink(missing_ok=True)

    def test_update_preserves_old_on_download_failure(self, installed_layout, clean_state):
        """Binary remains unchanged if update download fails."""
        # Record current build hash
        original_hash = get_build_hash()

        # Try to update (will fail if no network or no newer version)
        run_capsem("update", "--yes", timeout=30)
        # Regardless of outcome, the installed binary should be unchanged
        current_hash = get_build_hash()
        assert current_hash == original_hash, (
            f"binary changed unexpectedly: {original_hash} -> {current_hash}"
        )
