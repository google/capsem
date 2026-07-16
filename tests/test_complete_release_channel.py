"""Contracts for complete stable + nightly static release-site assembly."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from urllib.error import URLError

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _module():
    path = PROJECT_ROOT / "scripts" / "build-complete-release-channel.py"
    spec = importlib.util.spec_from_file_location("build_complete_release_channel", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _legacy() -> dict[str, object]:
    return {"format": "1.5", "assets": {}, "binaries": {}}


def _graph(channel: str) -> dict[str, object]:
    return {"version": "1.0.1", "channel": channel, "profiles": {}, "packages": []}


def test_missing_public_channel_bootstraps_from_primary_asset_manifest(monkeypatch) -> None:
    module = _module()

    def fake_read(source: str):
        if source == "source.json":
            return _legacy()
        raise URLError("missing")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, documents = module.resolve_channel_sources(
        explicit={"stable": "source.json"},
        primary_channel="stable",
        release_site="https://release.example",
        allow_mirror_missing=True,
    )

    assert sources == {"stable": "source.json", "nightly": "source.json"}
    assert documents["nightly"] == _legacy()


def test_existing_other_channel_is_preserved_from_public_graph(monkeypatch) -> None:
    module = _module()
    nightly_url = "https://release.example/assets/nightly/manifest.json"

    def fake_read(source: str):
        if source == "source.json":
            return _legacy()
        if source == nightly_url:
            return _graph("nightly")
        raise AssertionError(source)

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, documents = module.resolve_channel_sources(
        explicit={"stable": "source.json"},
        primary_channel="stable",
        release_site="https://release.example",
        allow_mirror_missing=True,
    )

    assert sources["nightly"] == nightly_url
    assert documents["nightly"]["channel"] == "nightly"


def test_graph_manifest_cannot_be_relabelled_to_bootstrap_another_channel(monkeypatch) -> None:
    module = _module()

    def fake_read(source: str):
        if source == "stable.json":
            return _graph("stable")
        raise URLError("missing")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    with pytest.raises(RuntimeError, match="cannot bootstrap missing nightly"):
        module.resolve_channel_sources(
            explicit={"stable": "stable.json"},
            primary_channel="stable",
            release_site="https://release.example",
            allow_mirror_missing=True,
        )


def test_missing_public_channel_fails_closed_without_explicit_bootstrap(monkeypatch) -> None:
    module = _module()

    def fake_read(source: str):
        if source == "source.json":
            return _legacy()
        raise URLError("missing")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    with pytest.raises(RuntimeError, match="cannot preserve required nightly"):
        module.resolve_channel_sources(
            explicit={"stable": "source.json"},
            primary_channel="stable",
            release_site="https://release.example",
            allow_mirror_missing=False,
        )


def test_asset_workflow_and_local_gate_share_complete_dist_builder() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    release = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    local_web_gate = (PROJECT_ROOT / "scripts/check-web-surface.sh").read_text()
    builder = (PROJECT_ROOT / "scripts/build-complete-release-channel.py").read_text()

    assert "scripts/build-complete-release-channel.py" in workflow
    assert "scripts/build-complete-release-channel.py" in release
    assert "scripts/build-complete-release-channel.py" in local_web_gate
    assert local_web_gate.count("scripts/build-complete-release-channel.py") == 2
    assert '--channel-source "stable=file://$graph_sources/stable.json"' in local_web_gate
    assert '--channel-source "nightly=file://$graph_sources/nightly.json"' in local_web_gate
    assert "--profile-source-ref HEAD" in local_web_gate
    assert "--channel stable" not in workflow.split(
        "- name: Build complete asset channel preview", maxsplit=1
    )[1].split("- name: Publish immutable", maxsplit=1)[0]
    assert 'REQUIRED_CHANNELS = ("stable", "nightly")' in builder
    assert '"assets",\n                "channel",\n                "check"' in builder
