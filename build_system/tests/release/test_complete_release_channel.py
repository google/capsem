"""Contracts for complete stable + nightly static release-site assembly."""

from __future__ import annotations

import importlib
from email.message import Message
from pathlib import Path
from urllib.error import HTTPError, URLError

import pytest
from capsem_builder.release.tools import (
    build_complete_release_channel,
    check_channel_deploy_freshness,
)

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _module():
    return importlib.reload(build_complete_release_channel)


def _freshness_module():
    return check_channel_deploy_freshness


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


def test_nightly_preserves_latest_good_stable_from_public_graph(monkeypatch) -> None:
    module = _module()
    stable_url = "https://release.example/assets/stable/manifest.json"

    def fake_read(source: str):
        if source == "nightly.json":
            return _graph("nightly")
        if source == stable_url:
            return _graph("stable")
        raise AssertionError(source)

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, documents = module.resolve_channel_sources(
        explicit={"nightly": "nightly.json"},
        primary_channel="nightly",
        release_site="https://release.example",
        allow_mirror_missing=False,
    )

    assert sources["stable"] == stable_url
    assert documents["stable"]["channel"] == "stable"


def test_stable_graph_never_reads_nightly(monkeypatch) -> None:
    module = _module()

    def fake_read(source: str):
        if source == "stable.json":
            return _graph("stable")
        raise AssertionError(f"stable publication read unrelated channel: {source}")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, documents = module.resolve_channel_sources(
        explicit={"stable": "stable.json"},
        primary_channel="stable",
        release_site="https://release.example",
        allow_mirror_missing=False,
    )

    assert sources == {"stable": "stable.json"}
    assert set(documents) == {"stable"}


def test_missing_public_channel_fails_closed_without_explicit_bootstrap(monkeypatch) -> None:
    """Unreachable stays fatal, which is the protection worth keeping.

    A channel that answers and is not a graph has never published, and is now
    assembled without. A channel that cannot be reached says nothing about
    whether it exists -- treating a blip as "unpublished" is how a nightly that
    does exist gets dropped by the release that never saw it.
    """
    module = _module()

    def fake_read(source: str):
        if source == "nightly.json":
            return _graph("nightly")
        raise URLError("missing")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    with pytest.raises(RuntimeError, match="cannot reach stable channel"):
        module.resolve_channel_sources(
            explicit={"nightly": "nightly.json"},
            primary_channel="nightly",
            release_site="https://release.example",
            allow_mirror_missing=False,
        )


def test_asset_workflow_and_local_gate_share_complete_dist_builder() -> None:
    module = _module()
    workflow = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text()
    release = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()
    local_web_gate = (PROJECT_ROOT / "build_system/scripts/web/check-web-surface.sh").read_text()
    builder = Path(build_complete_release_channel.__file__).read_text()

    assert "build_system/scripts/release/build-complete-release-channel.py" in workflow
    assert "build_system/scripts/release/build-complete-release-channel.py" in release
    assert "build_system/scripts/release/build-complete-release-channel.py" in local_web_gate
    assert local_web_gate.count("build_system/scripts/release/build-complete-release-channel.py") == 2
    assert '--channel-source "stable=file://$graph_sources/stable.json"' in local_web_gate
    assert '--channel-source "nightly=file://$graph_sources/nightly.json"' in local_web_gate
    assert '--profile-source-root "$ROOT"' in local_web_gate
    assert "--profile-source-ref HEAD" not in local_web_gate
    assert 'command.extend(["--source-root", str(args.profile_source_root)])' in builder
    assert "profile_source = parser.add_mutually_exclusive_group()" in builder
    assert (
        "--channel stable"
        not in workflow.split("- name: Build complete asset channel preview", maxsplit=1)[1].split(
            "- name: Publish immutable", maxsplit=1
        )[0]
    )
    assert module.REQUIRED_CHANNELS == ("stable", "nightly")
    assert '"assets",\n                "channel",\n                "check"' in builder


def test_complete_builder_preserves_public_mirror_from_public_bytes() -> None:
    builder = Path(build_complete_release_channel.__file__).read_text()

    assert "is_public_mirror" in builder
    assert 'command.extend(["--public-base", args.release_site])' in builder


def test_stable_assembly_does_not_depend_on_a_404_nightly(monkeypatch) -> None:
    """Stable never reads nightly, including when the public path is a 404."""
    module = _module()

    def fake_read(source: str):
        if source == "source.json":
            return _graph("stable")
        raise HTTPError(source, 404, "Not Found", Message(), None)

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, documents = module.resolve_channel_sources(
        explicit={"stable": "source.json"},
        primary_channel="stable",
        release_site="https://release.example",
        allow_mirror_missing=False,
    )

    assert sources == {"stable": "source.json"}, "nothing to preserve means preserve nothing"
    assert "nightly" not in documents


def test_stable_deploy_never_reads_nightly(tmp_path: Path, monkeypatch) -> None:
    module = _freshness_module()
    dist = tmp_path / "dist"
    (dist / "assets" / "stable").mkdir(parents=True)

    def unrelated(_release_site: str, channel: str) -> bytes:
        raise AssertionError(f"stable deployment read unrelated channel: {channel}")

    monkeypatch.setattr(module, "read_live_manifest", unrelated)
    module.verify_untouched_channels(
        selected_channel="stable",
        dist=dist,
        release_site="https://release.example.test",
    )


def test_nightly_requires_a_latest_good_stable_graph(monkeypatch) -> None:
    """Nightly owns the dependency: channel switching starts from stable."""
    module = _module()

    def fake_read(source: str):
        if source == "nightly.json":
            return _graph("nightly")
        raise ValueError("public preserved manifest is not a release graph")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    with pytest.raises(RuntimeError, match="latest good stable"):
        module.resolve_channel_sources(
            explicit={"nightly": "nightly.json"},
            primary_channel="nightly",
            release_site="https://release.example",
            allow_mirror_missing=False,
        )


def test_only_the_channels_that_resolved_are_built_and_checked() -> None:
    """The build loop and the check loop must agree on the same list.

    They disagreed once: the build skipped an unpublished nightly and the check
    still demanded it, so `assets channel check --channel nightly` failed after
    the primary channel had already been assembled and reported valid. Both read
    this now, which is why it is a function rather than a comprehension written
    twice.
    """
    module = _module()

    assert module.channels_to_assemble({"stable": {}}, "stable") == ["stable"]
    assert module.channels_to_assemble({"stable": {}, "nightly": {}}, "stable") == [
        "nightly",
        "stable",
    ]
    # The primary is always last, so a preserved channel is built against a
    # graph the primary has not yet replaced.
    assert module.channels_to_assemble({"stable": {}, "nightly": {}}, "nightly")[-1] == "nightly"


def test_both_loops_read_the_same_channel_list() -> None:
    """Stated in the source, because the failure was the two drifting apart."""
    source = Path(build_complete_release_channel.__file__).read_text(encoding="utf-8")

    assert source.count("channels_to_assemble(") == 2, (
        "one definition and one call: a second inline list is how the build and "
        "the check disagreed about nightly"
    )
    assert source.count("for channel in build_order:") == 2, (
        "the build loop and the check loop must both iterate the resolved list"
    )
