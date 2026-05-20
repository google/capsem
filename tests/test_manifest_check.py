from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

import blake3
from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.manifest_check import (
    ManifestCheckReport,
    check_profile_manifest_fast,
    dump_manifest_check_report_json,
)
from capsem.builder.profiles import create_profile_draft, dump_profile_json


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
