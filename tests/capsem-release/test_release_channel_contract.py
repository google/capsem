"""Local release-channel contract tests.

These tests build the generated release-channel dist with capsem-admin, serve it
with Cloudflare Pages _headers semantics, and run the public release-site
validator against the local URL.
"""

from __future__ import annotations

import contextlib
import functools
import http.server
import importlib.util
import json
import os
import shutil
import socketserver
import subprocess
import sys
import threading
from collections.abc import Iterator
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import pytest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
CHANNEL = "stable"

pytestmark = pytest.mark.build_chain


def _run(
    command: list[str],
    *,
    timeout: int = 180,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
        env={**os.environ, **env} if env else None,
    )
    assert result.returncode == 0, (
        f"command failed: {' '.join(command)}\n"
        f"stdout:\n{result.stdout}\n"
        f"stderr:\n{result.stderr}"
    )
    return result


def _run_admin(
    *args: str,
    timeout: int = 180,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return _run(
        ["cargo", "run", "-p", "capsem-admin", "--quiet", "--", *args],
        timeout=timeout,
        env=env,
    )


def _load_release_validator() -> Any:
    module_path = PROJECT_ROOT / "scripts" / "check-release-site-contract.py"
    spec = importlib.util.spec_from_file_location("check_release_site_contract", module_path)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _build_release_channel(
    dist: Path,
    *,
    manifest_path: Path | None = None,
    assets_dir: Path | None = None,
) -> None:
    manifest_url = (manifest_path or PROJECT_ROOT / "assets" / "manifest.json").resolve().as_uri()
    assets_dir = assets_dir or PROJECT_ROOT / "assets"
    _run_admin(
        "assets",
        "channel",
        "build",
        "--manifest",
        manifest_url,
        "--assets-dir",
        str(assets_dir),
        "--channel",
        CHANNEL,
        "--out-dir",
        str(dist),
    )
    _run(
        ["pnpm", "--dir", "release-site", "install", "--frozen-lockfile"],
        timeout=180,
        env={},
    )
    _run(
        ["pnpm", "--dir", "release-site", "run", "build:channel"],
        timeout=180,
        env={"CAPSEM_RELEASE_CHANNEL_DIST": str(dist)},
    )
    _run_admin("assets", "channel", "check", "--channel", CHANNEL, "--dist", str(dist))


@pytest.fixture(scope="module")
def release_channel_dist(tmp_path_factory: pytest.TempPathFactory) -> Path:
    dist = tmp_path_factory.mktemp("release-channel") / "dist"
    _build_release_channel(dist)
    return dist


def _headers_rules(dist: Path) -> list[tuple[str, dict[str, str]]]:
    rules: list[tuple[str, dict[str, str]]] = []
    current_path: str | None = None
    current_headers: dict[str, str] = {}
    for raw_line in (dist / "_headers").read_text().splitlines():
        if not raw_line.strip() or raw_line.lstrip().startswith("#"):
            continue
        if raw_line.startswith((" ", "\t")):
            name, value = raw_line.strip().split(":", maxsplit=1)
            current_headers[name.strip()] = value.strip()
            continue
        if current_path is not None:
            rules.append((current_path, current_headers))
        current_path = raw_line.strip()
        current_headers = {}
    if current_path is not None:
        rules.append((current_path, current_headers))
    assert rules, "generated release channel must include Cloudflare _headers rules"
    return rules


def _headers_for_path(path: str, rules: list[tuple[str, dict[str, str]]]) -> dict[str, str]:
    selected: dict[str, tuple[int, str]] = {}
    for pattern, rule_headers in rules:
        specificity = _header_rule_specificity(pattern, path)
        if specificity is None:
            continue
        for name, value in rule_headers.items():
            previous = selected.get(name)
            if previous is None or specificity >= previous[0]:
                selected[name] = (specificity, value)
    return {name: value for name, (_specificity, value) in selected.items()}


def _header_rule_specificity(pattern: str, path: str) -> int | None:
    if pattern.endswith("*"):
        prefix = pattern[:-1]
        if path.startswith(prefix):
            return len(prefix)
        return None
    if path == pattern:
        return len(pattern) + 1000
    return None


@contextlib.contextmanager
def _serve_release_channel(
    dist: Path,
    *,
    header_overrides: dict[str, str] | None = None,
) -> Iterator[str]:
    rules = _headers_rules(dist)
    overrides = header_overrides or {}

    class ReleaseChannelHandler(http.server.SimpleHTTPRequestHandler):
        def end_headers(self) -> None:
            request_path = urlparse(self.path).path
            headers = _headers_for_path(request_path, rules)
            headers.update(overrides)
            for name, value in headers.items():
                self.send_header(name, value)
            super().end_headers()

        def log_message(self, format: str, *args: Any) -> None:
            return

    handler = functools.partial(ReleaseChannelHandler, directory=str(dist))
    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler) as server:
        server.allow_reuse_address = True
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            host, port = server.server_address
            yield f"http://{host}:{port}"
        finally:
            server.shutdown()
            thread.join(timeout=5)


def _validate_release_site(url: str, *, capsys: pytest.CaptureFixture[str]) -> int:
    validator = _load_release_validator()
    exit_code = validator.validate_release_site(
        release_site=url,
        channel=CHANNEL,
        attempts=1,
        delay_seconds=0,
    )
    captured = capsys.readouterr()
    if exit_code != 0:
        pytest.fail(
            f"release-site validator failed for {url}\n"
            f"stdout:\n{captured.out}\n"
            f"stderr:\n{captured.err}"
        )
    return exit_code


def _validator_exit_code(url: str, *, capsys: pytest.CaptureFixture[str]) -> tuple[int, str, str]:
    validator = _load_release_validator()
    exit_code = validator.validate_release_site(
        release_site=url,
        channel=CHANNEL,
        attempts=1,
        delay_seconds=0,
    )
    captured = capsys.readouterr()
    return exit_code, captured.out, captured.err


def test_generated_release_channel_passes_public_contract(
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert (release_channel_dist / "index.html").is_file()
    assert (release_channel_dist / "channels.json").is_file()
    assert (release_channel_dist / "health.json").is_file()
    assert (release_channel_dist / "_headers").is_file()
    assert (release_channel_dist / "assets" / CHANNEL / "manifest.json").is_file()
    assert (release_channel_dist / "assets" / "releases").is_dir()
    assert (release_channel_dist / "profiles" / "releases").is_dir()
    channels = json.loads((release_channel_dist / "channels.json").read_text())
    assert channels["channels"][CHANNEL]["manifests"][0]["url"] == (
        f"/assets/{CHANNEL}/manifest.json"
    )
    assert channels["channels"][CHANNEL]["profile_catalog"]["source"].startswith(
        "/profiles/releases/"
    )

    with _serve_release_channel(release_channel_dist) as url:
        assert _validate_release_site(url, capsys=capsys) == 0


def test_fresh_install_assets_generate_release_channel_evidence(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assets_dir = tmp_path / "assets"
    dist = tmp_path / "dist"
    _run(
        ["bash", "scripts/prepare-install-test-assets.sh"],
        env={"CAPSEM_ASSETS_DIR": str(assets_dir)},
    )

    _build_release_channel(
        dist,
        manifest_path=assets_dir / "manifest.json",
        assets_dir=assets_dir,
    )

    health = json.loads((dist / "health.json").read_text())
    vm_oboms = health["evidence"]["vm_oboms"]
    attestations = health["evidence"]["attestations"]
    assert vm_oboms
    assert vm_oboms[0]["url"].endswith("-obom.cdx.json")
    vm_attestation = next(
        item for item in attestations if item["name"] == "github_attestations_vm_assets"
    )
    assert vm_attestation["predicate_url"] == vm_oboms[0]["url"]

    with _serve_release_channel(dist) as url:
        assert _validate_release_site(url, capsys=capsys) == 0


def test_release_channel_contract_rejects_swapped_manifest(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    dist = tmp_path / "dist"
    shutil.copytree(release_channel_dist, dist)
    manifest_path = dist / "assets" / CHANNEL / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["assets"]["current"] = "2030.0101.1"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")

    with _serve_release_channel(dist) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 1
    assert "channel manifest BLAKE3 mismatch" in stderr
    assert "channel asset version mismatch with manifest" in stderr


def test_release_channel_contract_ignores_stale_health_summary(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    dist = tmp_path / "dist"
    shutil.copytree(release_channel_dist, dist)
    health_path = dist / "health.json"
    health = json.loads(health_path.read_text())
    health["current"]["assets"] = "2030.0101.1"
    health["assets"]["version"] = "2030.0101.1"
    health_path.write_text(json.dumps(health, indent=2, sort_keys=True) + "\n")

    with _serve_release_channel(dist) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 0
    assert "health" not in stderr.lower()


def test_release_channel_contract_rejects_cache_header_drift(
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    with _serve_release_channel(
        release_channel_dist,
        header_overrides={"Cache-Control": "public, max-age=3600"},
    ) as url:
        exit_code, _stdout, stderr = _validator_exit_code(url, capsys=capsys)

    assert exit_code == 1
    assert "Cache-Control must contain no-cache" in stderr


def test_two_generated_release_channels_have_same_machine_contract(
    tmp_path: Path,
    release_channel_dist: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    second_dist = tmp_path / "second-dist"
    _build_release_channel(second_dist)

    first_manifest = (
        release_channel_dist / "assets" / CHANNEL / "manifest.json"
    ).read_text()
    second_manifest = (second_dist / "assets" / CHANNEL / "manifest.json").read_text()
    assert second_manifest == first_manifest
    assert (second_dist / "_headers").read_text() == (release_channel_dist / "_headers").read_text()

    for dist in (release_channel_dist, second_dist):
        with _serve_release_channel(dist) as url:
            assert _validate_release_site(url, capsys=capsys) == 0
