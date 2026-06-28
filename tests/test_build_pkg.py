"""Artifact-level tests for scripts/build-pkg.sh."""

import contextlib
import functools
import http.server
import json
import plistlib
import shutil
import subprocess
import threading
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SCRIPT = REPO_ROOT / "scripts" / "build-pkg.sh"

REQUIRED_BINARIES = [
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-tui",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
    "capsem-admin",
]

pytestmark = pytest.mark.skipif(
    shutil.which("pkgutil") is None
    or shutil.which("pkgbuild") is None
    or shutil.which("productbuild") is None,
    reason="macOS package tools not available",
)


def _seed_app(app: Path) -> None:
    contents = app / "Contents"
    macos = contents / "MacOS"
    macos.mkdir(parents=True)
    (macos / "capsem-app").write_text("#!/bin/sh\nexit 0\n")
    (macos / "capsem-app").chmod(0o755)
    (contents / "Info.plist").write_bytes(
        plistlib.dumps(
            {
                "CFBundleExecutable": "capsem-app",
                "CFBundleIdentifier": "org.capsem.test",
                "CFBundleName": "Capsem",
                "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": "0.0.0",
                "CFBundleVersion": "0",
            }
        )
    )


def _seed_binaries(bin_dir: Path) -> None:
    bin_dir.mkdir(parents=True)
    for name in REQUIRED_BINARIES:
        path = bin_dir / name
        path.write_text(f"#!/bin/sh\necho {name}\n")
        path.chmod(0o755)


def _seed_config(config_dir: Path) -> None:
    profile = config_dir / "profiles" / "code"
    profile.mkdir(parents=True)
    (profile / "profile.toml").write_text("id = \"code\"\n")
    (profile / "enforcement.toml").write_text("# enforcement\n")


def _seed_manifest_and_local_assets(manifest: Path, assets_dir: Path) -> None:
    digest = "b" * 64
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "version": "9.9.9-test",
                "assets": {
                    "current": "test-release",
                    "releases": {
                        "test-release": {
                            "arches": {
                                "arm64": {"rootfs.erofs": {"hash": digest}},
                                "x86_64": {"rootfs.erofs": {"hash": digest}},
                            }
                        }
                    },
                },
                "binaries": {},
            },
            sort_keys=True,
        )
        + "\n"
    )
    for arch in ("arm64", "x86_64"):
        arch_dir = assets_dir / arch
        arch_dir.mkdir(parents=True)
        (arch_dir / f"rootfs-{digest[:16]}.erofs").write_bytes(b"fake-rootfs")


def _find_capsem_share(expanded_pkg: Path) -> Path:
    matches = list(expanded_pkg.rglob("usr/local/share/capsem"))
    assert len(matches) == 1, f"expected one capsem share payload, found {matches}"
    return matches[0]


class _QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: object) -> None:
        return


@contextlib.contextmanager
def _serve_directory(root: Path):
    handler = functools.partial(_QuietHandler, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def test_macos_pkg_payload_is_closed_and_manifest_only_for_assets(tmp_path: Path) -> None:
    app = tmp_path / "Capsem.app"
    bin_dir = tmp_path / "bin"
    assets_dir = tmp_path / "assets"
    config_dir = tmp_path / "target-config"
    manifest = tmp_path / "manifest.json"

    _seed_app(app)
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    _seed_manifest_and_local_assets(manifest, assets_dir)

    version = "9.9.9-test"
    output_pkg = REPO_ROOT / "packages" / f"Capsem-{version}.pkg"
    output_pkg.unlink(missing_ok=True)
    try:
        res = subprocess.run(
            [
                str(SCRIPT),
                "--manifest",
                manifest.resolve().as_uri(),
                str(app),
                str(bin_dir),
                str(assets_dir),
                str(config_dir),
                version,
            ],
            cwd=tmp_path,
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert res.returncode == 0, (
            f"build-pkg.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"
        )
        assert output_pkg.is_file()

        expanded = tmp_path / "expanded"
        subprocess.run(
            ["pkgutil", "--expand-full", str(output_pkg), str(expanded)],
            check=True,
            capture_output=True,
            text=True,
        )
        share = _find_capsem_share(expanded)
        assert list(expanded.rglob("Applications/Capsem.app")), (
            "Capsem.app missing from package payload"
        )

        assets = share / "assets"
        assert sorted(path.name for path in assets.iterdir()) == [
            "manifest-origin.json",
            "manifest.json",
        ]
        origin = json.loads((assets / "manifest-origin.json").read_text())
        assert origin["schema"] == "capsem.manifest_origin.v1"
        assert origin["origin"] == "package"
        assert origin["source"] == manifest.resolve().as_uri()
        assert "packaged_at" in origin

        for name in REQUIRED_BINARIES:
            assert (share / "bin" / name).is_file()
        assert (share / "profiles" / "code" / "profile.toml").is_file()

        unexpected = []
        for path in share.rglob("*"):
            rel = path.relative_to(share).as_posix()
            if path.is_dir():
                continue
            if rel.startswith("bin/") and rel.removeprefix("bin/") in REQUIRED_BINARIES:
                continue
            if rel in {"assets/manifest.json", "assets/manifest-origin.json"}:
                continue
            if rel.startswith("profiles/"):
                continue
            if rel == "entitlements.plist":
                continue
            unexpected.append(rel)

        assert unexpected == []
    finally:
        output_pkg.unlink(missing_ok=True)


def test_macos_pkg_rejects_retired_keychain_payload_binaries(tmp_path: Path) -> None:
    app = tmp_path / "Capsem.app"
    bin_dir = tmp_path / "bin"
    assets_dir = tmp_path / "assets"
    config_dir = tmp_path / "target-config"
    manifest = tmp_path / "manifest.json"

    _seed_app(app)
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    _seed_manifest_and_local_assets(manifest, assets_dir)
    (bin_dir / "capsem-service").write_text(
        "#!/bin/sh\n"
        "echo 'stale helper still opens org.capsem.credentials from the default keychain'\n"
    )
    (bin_dir / "capsem-service").chmod(0o755)

    version = "9.9.11-keychain-test"
    output_pkg = REPO_ROOT / "packages" / f"Capsem-{version}.pkg"
    output_pkg.unlink(missing_ok=True)
    try:
        res = subprocess.run(
            [
                str(SCRIPT),
                "--manifest",
                manifest.resolve().as_uri(),
                str(app),
                str(bin_dir),
                str(assets_dir),
                str(config_dir),
                version,
            ],
            cwd=tmp_path,
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert res.returncode != 0
        combined = res.stdout + res.stderr
        assert "retired native Keychain credential-store marker" in combined
        assert "capsem-service" in combined
        assert "org.capsem.credentials" in combined
        assert not output_pkg.exists()
    finally:
        output_pkg.unlink(missing_ok=True)


def test_macos_pkg_rejects_bare_manifest_path(tmp_path: Path) -> None:
    app = tmp_path / "Capsem.app"
    bin_dir = tmp_path / "bin"
    assets_dir = tmp_path / "assets"
    config_dir = tmp_path / "target-config"
    manifest = tmp_path / "manifest.json"

    _seed_app(app)
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    _seed_manifest_and_local_assets(manifest, assets_dir)

    res = subprocess.run(
        [
            str(SCRIPT),
            "--manifest",
            str(manifest),
            str(app),
            str(bin_dir),
            str(assets_dir),
            str(config_dir),
            "9.9.12-bare-path-test",
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert res.returncode != 0
    assert "manifest must be a URL" in (res.stdout + res.stderr)


def test_macos_pkg_remote_manifest_override_records_source_and_payload(tmp_path: Path) -> None:
    app = tmp_path / "Capsem.app"
    bin_dir = tmp_path / "bin"
    assets_dir = tmp_path / "assets"
    config_dir = tmp_path / "target-config"
    manifest_root = tmp_path / "remote"
    manifest = manifest_root / "corp-manifest.json"

    _seed_app(app)
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    manifest_root.mkdir()
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "version": "remote-test",
                "assets": {"current": "corp", "releases": {"corp": {"arches": {}}}},
                "binaries": {"current": "remote"},
            },
            sort_keys=True,
        )
        + "\n"
    )
    assets_dir.mkdir()

    version = "9.9.10-remote-test"
    output_pkg = REPO_ROOT / "packages" / f"Capsem-{version}.pkg"
    output_pkg.unlink(missing_ok=True)
    try:
        with _serve_directory(manifest_root) as base_url:
            manifest_url = f"{base_url}/corp-manifest.json"
            res = subprocess.run(
                [
                    str(SCRIPT),
                    "--manifest",
                    manifest_url,
                    str(app),
                    str(bin_dir),
                    str(assets_dir),
                    str(config_dir),
                    version,
                ],
                cwd=tmp_path,
                capture_output=True,
                text=True,
                timeout=60,
            )
        assert res.returncode == 0, (
            f"build-pkg.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"
        )
        assert output_pkg.is_file()

        expanded = tmp_path / "expanded-remote"
        subprocess.run(
            ["pkgutil", "--expand-full", str(output_pkg), str(expanded)],
            check=True,
            capture_output=True,
            text=True,
        )
        assets = _find_capsem_share(expanded) / "assets"
        assert sorted(path.name for path in assets.iterdir()) == [
            "manifest-origin.json",
            "manifest.json",
        ]
        assert (assets / "manifest.json").read_text() == manifest.read_text()
        origin = json.loads((assets / "manifest-origin.json").read_text())
        assert origin["schema"] == "capsem.manifest_origin.v1"
        assert origin["origin"] == "package"
        assert origin["source"] == manifest_url
        assert "packaged_at" in origin
    finally:
        output_pkg.unlink(missing_ok=True)
