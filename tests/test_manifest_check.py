from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

import blake3
from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.manifest_check import (
    ManifestCheckReport,
    check_profile_manifest_download,
    check_profile_manifest_fast,
    dump_manifest_check_report_json,
)
from capsem.builder.profiles import (
    ArchAssets,
    AssetDeclaration,
    create_profile_draft,
    dump_profile_json,
)


def _blake3(payload: bytes) -> str:
    return f"blake3:{blake3.blake3(payload).hexdigest()}"


def _profile_manifest_json(
    *,
    profile_url: str,
    profile_hash: str,
    signature_url: str,
) -> str:
    return f"""
    {{
      "format": 1,
      "profiles": {{
        "corp-dev": {{
          "current_revision": "2026.0520.13",
          "revisions": {{
            "2026.0520.13": {{
              "status": "active",
              "min_binary": "1.0.0",
              "profile_url": "{profile_url}",
              "profile_hash": "{profile_hash}",
              "profile_signature_url": "{signature_url}"
            }}
          }}
        }}
      }}
    }}
    """


def _asset(url: str, payload: bytes) -> AssetDeclaration:
    return AssetDeclaration(
        url=url,
        hash=_blake3(payload),
        signature_url=f"{url}.minisig",
        size=len(payload),
        content_type="application/octet-stream",
    )


def test_fast_manifest_check_accepts_local_file_payload_and_signature(
    tmp_path: Path,
) -> None:
    profile = create_profile_draft("corp-dev", revision="2026.0520.13")
    profile_payload = dump_profile_json(profile).encode()
    profile_path = tmp_path / "corp-dev.profile.json"
    profile_path.write_bytes(profile_payload)
    signature_path = tmp_path / "corp-dev.profile.json.minisig"
    signature_path.write_text("trusted signature placeholder\n", encoding="utf-8")
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        _profile_manifest_json(
            profile_url=profile_path.as_uri(),
            profile_hash=_blake3(profile_payload),
            signature_url=signature_path.as_uri(),
        ),
        encoding="utf-8",
    )

    report = check_profile_manifest_fast(manifest_path)
    dumped = dump_manifest_check_report_json(report)
    reparsed = ManifestCheckReport.model_validate_json(dumped)

    assert report == reparsed
    assert report.ok is True
    assert report.profiles[0].profile_id == "corp-dev"
    assert {check.kind for check in report.profiles[0].checks} == {
        "profile_payload",
        "profile_signature",
    }


def test_fast_manifest_check_reports_local_profile_hash_mismatch(
    tmp_path: Path,
) -> None:
    profile = create_profile_draft("corp-dev", revision="2026.0520.13")
    profile_path = tmp_path / "corp-dev.profile.json"
    profile_path.write_text(dump_profile_json(profile), encoding="utf-8")
    signature_path = tmp_path / "corp-dev.profile.json.minisig"
    signature_path.write_text("trusted signature placeholder\n", encoding="utf-8")
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        _profile_manifest_json(
            profile_url=profile_path.as_uri(),
            profile_hash="blake3:" + "a" * 64,
            signature_url=signature_path.as_uri(),
        ),
        encoding="utf-8",
    )

    report = check_profile_manifest_fast(manifest_path)

    assert report.ok is False
    failed = [check for item in report.profiles for check in item.checks if not check.ok]
    assert len(failed) == 1
    assert failed[0].failure == "hash_mismatch"


def test_capsem_admin_manifest_check_fast_uses_http_head(tmp_path: Path) -> None:
    seen_methods: list[tuple[str, str]] = []

    class Handler(BaseHTTPRequestHandler):
        def do_HEAD(self) -> None:
            seen_methods.append(("HEAD", self.path))
            if self.path == "/corp-dev.profile.json":
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", "1234")
                self.end_headers()
                return
            if self.path == "/corp-dev.profile.json.minisig":
                self.send_response(200)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Length", "128")
                self.end_headers()
                return
            self.send_response(404)
            self.end_headers()

        def log_message(self, format: str, *args: object) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base_url = f"http://127.0.0.1:{server.server_port}"
        manifest_path = tmp_path / "manifest.json"
        manifest_path.write_text(
            _profile_manifest_json(
                profile_url=f"{base_url}/corp-dev.profile.json",
                profile_hash="blake3:" + "b" * 64,
                signature_url=f"{base_url}/corp-dev.profile.json.minisig",
            ),
            encoding="utf-8",
        )

        result = CliRunner().invoke(
            cli,
            ["manifest", "check", str(manifest_path), "--fast", "--json"],
        )
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)

    assert result.exit_code == 0
    assert '"schema": "capsem.manifest-check.v1"' in result.output
    assert '"ok": true' in result.output
    assert '"status_code": 200' in result.output
    assert seen_methods == [
        ("HEAD", "/corp-dev.profile.json"),
        ("HEAD", "/corp-dev.profile.json.minisig"),
    ]


def test_capsem_admin_manifest_check_fast_returns_nonzero_on_missing_signature(
    tmp_path: Path,
) -> None:
    profile = create_profile_draft("corp-dev", revision="2026.0520.13")
    profile_payload = dump_profile_json(profile).encode()
    profile_path = tmp_path / "corp-dev.profile.json"
    profile_path.write_bytes(profile_payload)
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        _profile_manifest_json(
            profile_url=profile_path.as_uri(),
            profile_hash=_blake3(profile_payload),
            signature_url=(tmp_path / "missing.minisig").as_uri(),
        ),
        encoding="utf-8",
    )

    result = CliRunner().invoke(
        cli,
        ["manifest", "check", str(manifest_path), "--fast", "--json"],
    )

    assert result.exit_code == 1
    assert '"ok": false' in result.output
    assert '"failure": "missing"' in result.output


def test_download_manifest_check_fetches_profile_assets_and_signatures(
    tmp_path: Path,
) -> None:
    seen_methods: list[tuple[str, str]] = []
    blobs: dict[str, bytes] = {
        "/arm64/vmlinuz": b"kernel-arm64",
        "/arm64/vmlinuz.minisig": b"kernel signature\n",
        "/arm64/initrd.img": b"initrd-arm64",
        "/arm64/initrd.img.minisig": b"initrd signature\n",
        "/arm64/rootfs.squashfs": b"rootfs-arm64",
        "/arm64/rootfs.squashfs.minisig": b"rootfs signature\n",
        "/corp-dev.profile.json.minisig": b"profile signature\n",
    }

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            seen_methods.append(("GET", self.path))
            payload = blobs.get(self.path)
            if payload is None:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Content-Type", "application/octet-stream")
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, format: str, *args: object) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base_url = f"http://127.0.0.1:{server.server_port}"
        profile = create_profile_draft("corp-dev", revision="2026.0520.13")
        profile = profile.model_copy(
            update={
                "vm": profile.vm.model_copy(
                    update={
                        "assets": {
                            "arm64": ArchAssets(
                                kernel=_asset(
                                    f"{base_url}/arm64/vmlinuz",
                                    blobs["/arm64/vmlinuz"],
                                ),
                                initrd=_asset(
                                    f"{base_url}/arm64/initrd.img",
                                    blobs["/arm64/initrd.img"],
                                ),
                                rootfs=_asset(
                                    f"{base_url}/arm64/rootfs.squashfs",
                                    blobs["/arm64/rootfs.squashfs"],
                                ),
                            )
                        }
                    }
                )
            }
        )
        profile_payload = dump_profile_json(profile).encode()
        blobs["/corp-dev.profile.json"] = profile_payload
        manifest_path = tmp_path / "manifest.json"
        manifest_path.write_text(
            _profile_manifest_json(
                profile_url=f"{base_url}/corp-dev.profile.json",
                profile_hash=_blake3(profile_payload),
                signature_url=f"{base_url}/corp-dev.profile.json.minisig",
            ),
            encoding="utf-8",
        )

        report = check_profile_manifest_download(
            manifest_path,
            download_dir=tmp_path / "downloads",
        )
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)

    assert report.ok is True
    assert report.mode == "download"
    assert report.download_dir == str(tmp_path / "downloads")
    checks = report.profiles[0].checks
    assert [check.kind for check in checks].count("vm_asset") == 3
    assert [check.kind for check in checks].count("vm_asset_signature") == 3
    assert all(check.download_path for check in checks)
    assert ("GET", "/corp-dev.profile.json") in seen_methods
    assert ("GET", "/arm64/rootfs.squashfs") in seen_methods
    assert not any(method == "HEAD" for method, _ in seen_methods)


def test_capsem_admin_manifest_check_download_returns_nonzero_on_asset_mismatch(
    tmp_path: Path,
) -> None:
    assets_dir = tmp_path / "assets"
    assets_dir.mkdir()
    rootfs = assets_dir / "rootfs.squashfs"
    rootfs.write_bytes(b"tampered")
    (assets_dir / "rootfs.squashfs.minisig").write_text("signature\n", encoding="utf-8")
    kernel = assets_dir / "vmlinuz"
    kernel.write_bytes(b"kernel")
    (assets_dir / "vmlinuz.minisig").write_text("signature\n", encoding="utf-8")
    initrd = assets_dir / "initrd.img"
    initrd.write_bytes(b"initrd")
    (assets_dir / "initrd.img.minisig").write_text("signature\n", encoding="utf-8")

    profile = create_profile_draft("corp-dev", revision="2026.0520.13")
    profile = profile.model_copy(
        update={
            "vm": profile.vm.model_copy(
                update={
                    "assets": {
                        "arm64": ArchAssets(
                            kernel=_asset(kernel.as_uri(), kernel.read_bytes()),
                            initrd=_asset(initrd.as_uri(), initrd.read_bytes()),
                            rootfs=AssetDeclaration(
                                url=rootfs.as_uri(),
                                hash=_blake3(b"expected"),
                                signature_url=(
                                    assets_dir / "rootfs.squashfs.minisig"
                                ).as_uri(),
                                size=len(b"expected"),
                                content_type="application/octet-stream",
                            ),
                        )
                    }
                }
            )
        }
    )
    profile_payload = dump_profile_json(profile).encode()
    profile_path = tmp_path / "corp-dev.profile.json"
    profile_path.write_bytes(profile_payload)
    profile_signature = tmp_path / "corp-dev.profile.json.minisig"
    profile_signature.write_text("signature\n", encoding="utf-8")
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        _profile_manifest_json(
            profile_url=profile_path.as_uri(),
            profile_hash=_blake3(profile_payload),
            signature_url=profile_signature.as_uri(),
        ),
        encoding="utf-8",
    )

    result = CliRunner().invoke(
        cli,
        [
            "manifest",
            "check",
            str(manifest_path),
            "--download",
            "--download-dir",
            str(tmp_path / "downloads"),
            "--json",
        ],
    )

    assert result.exit_code == 1
    assert '"mode": "download"' in result.output
    assert '"kind": "vm_asset"' in result.output
    assert '"failure": "hash_mismatch"' in result.output


def test_download_manifest_check_reports_asset_size_mismatch(tmp_path: Path) -> None:
    assets_dir = tmp_path / "assets"
    assets_dir.mkdir()
    kernel = assets_dir / "vmlinuz"
    kernel.write_bytes(b"kernel")
    (assets_dir / "vmlinuz.minisig").write_text("signature\n", encoding="utf-8")
    initrd = assets_dir / "initrd.img"
    initrd.write_bytes(b"initrd")
    (assets_dir / "initrd.img.minisig").write_text("signature\n", encoding="utf-8")
    rootfs = assets_dir / "rootfs.squashfs"
    rootfs.write_bytes(b"too-large")
    (assets_dir / "rootfs.squashfs.minisig").write_text("signature\n", encoding="utf-8")

    profile = create_profile_draft("corp-dev", revision="2026.0520.13")
    profile = profile.model_copy(
        update={
            "vm": profile.vm.model_copy(
                update={
                    "assets": {
                        "arm64": ArchAssets(
                            kernel=_asset(kernel.as_uri(), kernel.read_bytes()),
                            initrd=_asset(initrd.as_uri(), initrd.read_bytes()),
                            rootfs=AssetDeclaration(
                                url=rootfs.as_uri(),
                                hash=_blake3(b"small"),
                                signature_url=(
                                    assets_dir / "rootfs.squashfs.minisig"
                                ).as_uri(),
                                size=len(b"small"),
                                content_type="application/octet-stream",
                            ),
                        )
                    }
                }
            )
        }
    )
    profile_payload = dump_profile_json(profile).encode()
    profile_path = tmp_path / "corp-dev.profile.json"
    profile_path.write_bytes(profile_payload)
    profile_signature = tmp_path / "corp-dev.profile.json.minisig"
    profile_signature.write_text("signature\n", encoding="utf-8")
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        _profile_manifest_json(
            profile_url=profile_path.as_uri(),
            profile_hash=_blake3(profile_payload),
            signature_url=profile_signature.as_uri(),
        ),
        encoding="utf-8",
    )

    report = check_profile_manifest_download(
        manifest_path,
        download_dir=tmp_path / "downloads",
    )

    failed = [check for item in report.profiles for check in item.checks if not check.ok]
    assert report.ok is False
    assert len(failed) == 1
    assert failed[0].kind == "vm_asset"
    assert failed[0].asset_kind == "rootfs"
    assert failed[0].failure == "size_mismatch"
