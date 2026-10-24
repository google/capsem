"""Canonical isolated manifest server/UserDir fixtures shared with installed tests."""

import http.server
import json
import os
import re
import shutil
import socketserver
import threading
import tomllib
from collections.abc import Callable
from contextlib import contextmanager
from pathlib import Path
from typing import ClassVar

REPO_ROOT = Path(__file__).resolve().parents[2]


def prepared_cli(path: Path) -> Path:
    """Use the existing selected cohort; never silently consume Cargo output."""
    from .constants import BIN_VARIABLE, host_bin_root
    if not os.environ.get(BIN_VARIABLE):
        raise RuntimeError('update check requires prepared CLI from CAPSEM_RELEASE_BIN_DIR')
    expected = host_bin_root() / 'capsem'
    if path != expected or path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
        raise RuntimeError('update check prepared CLI missing, foreign, symlinked or not executable')
    return path.resolve(strict=True)


class _HealthHandler(http.server.BaseHTTPRequestHandler):
    body: bytes = b""
    # Class-level on purpose: http.server builds a fresh handler per
    # request, so per-instance state would be unreachable. Each test makes
    # its own subclass through `type(...)` rather than mutating these.
    files: ClassVar[dict[str, bytes]] = {}
    requests: ClassVar[list[str]] = []

    def do_GET(self) -> None:
        self.requests.append(self.path)
        if self.path.endswith("/health.json"):
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
def _serve_manifest_release(
    files: dict[str, bytes] | Callable[[str], dict[str, bytes]],
):
    requests: list[str] = []
    handler = type(
        "ManifestReleaseHandler",
        (_HealthHandler,),
        {"body": b"", "files": files, "requests": requests},
    )
    server = socketserver.TCPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    base = f"http://127.0.0.1:{server.server_address[1]}"
    handler.files = files if isinstance(files, dict) else files(base)
    thread.start()
    try:
        yield base, requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)



def _update_release_graph(
    manifest: dict,
    binary_version: str,
    asset_version: str,
    *,
    min_binary: str | None,
    binary_files: list[dict] | None,
    asset_base: str | None,
) -> dict:
    """Build a release-graph update fixture from the manifest under test."""
    if asset_base is not None:
        manifest["asset_base"] = asset_base

    if binary_files:
        for package in manifest["packages"]:
            if package.get("status", "current") == "current":
                package["status"] = "supported"
        manifest["packages"].extend(
            _release_graph_package(binary_version, file) for file in binary_files
        )
    else:
        current_packages = [
            package
            for package in manifest["packages"]
            if package.get("status", "current") == "current"
        ]
        if not current_packages:
            raise AssertionError("release graph fixture has no current package")
        for package in current_packages:
            package["version"] = binary_version
            for binary in package.get("binaries", []):
                binary["version"] = binary_version

    runtime = manifest["runtime"]
    runtime["revision"] = asset_version
    if min_binary is not None:
        runtime["min_capsem_version"] = min_binary
    for architecture in runtime.get("architectures", []):
        architecture["package_inventory_revision"] = asset_version
        architecture["image_revision"] = asset_version
    return manifest



def _release_graph_package(binary_version: str, file: dict) -> dict:
    name = file["name"]
    if name.endswith(".pkg"):
        kind = "macos_pkg"
        platform_name = "macos"
        architecture = "arm64"
    elif name.endswith(".deb"):
        kind = "debian_package"
        platform_name = "linux"
        architecture = name.removesuffix(".deb").rsplit("_", 1)[-1]
        if architecture not in {"amd64", "arm64"}:
            raise AssertionError(f"unsupported Debian package architecture in {name}")
    else:
        raise AssertionError(f"unsupported native package fixture {name}")

    package_id = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return {
        "id": package_id,
        "name": name,
        "url": file["url"],
        "version": binary_version,
        "kind": kind,
        "platform": platform_name,
        "architecture": architecture,
        "status": "current",
        "bytes": file["size"],
        "digest": {
            "sha256": file["sha256"],
            "blake3": file["blake3"],
        },
        "binaries": [],
        "evidence": [],
    }



def _copy_user_dir_capsem(source_bin: Path, capsem_home: Path) -> Path:
    target = capsem_home / "bin" / "capsem"
    target.parent.mkdir(parents=True)
    shutil.copy2(source_bin, target)
    target.chmod(0o755)
    return target



def _write_manifest_metadata(capsem_home: Path, manifest_url: str) -> None:
    """Seed the same provenance document written by native package installers."""
    assets_dir = capsem_home / "assets"
    assets_dir.mkdir(parents=True, exist_ok=True)
    package_version = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))[
        "workspace"
    ]["package"]["version"]
    (assets_dir / "manifest-metadata.json").write_text(
        json.dumps(
            {
                "schema": "capsem.manifest_metadata.v1",
                "origin": "package",
                "manifest_url": manifest_url,
                "channel": "stable",
                "channel_kind": "public",
                "channel_locked": False,
                "packaged_at": "2026-07-17T00:00:00Z",
                "package_version": package_version,
            },
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
