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
    """Unreachable stays fatal, which is the protection worth keeping.

    A channel that answers and is not a graph has never published, and is now
    assembled without. A channel that cannot be reached says nothing about
    whether it exists -- treating a blip as "unpublished" is how a nightly that
    does exist gets dropped by the release that never saw it.
    """
    module = _module()

    def fake_read(source: str):
        if source == "source.json":
            return _legacy()
        raise URLError("missing")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    with pytest.raises(RuntimeError, match="cannot reach nightly channel"):
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
    assert '--profile-source-root "$ROOT"' in local_web_gate
    assert "--profile-source-ref HEAD" not in local_web_gate
    assert 'command.extend(["--source-root", str(args.profile_source_root)])' in builder
    assert "profile_source = parser.add_mutually_exclusive_group()" in builder
    assert "--channel stable" not in workflow.split(
        "- name: Build complete asset channel preview", maxsplit=1
    )[1].split("- name: Publish immutable", maxsplit=1)[0]
    assert 'REQUIRED_CHANNELS = ("stable", "nightly")' in builder
    assert '"assets",\n                "channel",\n                "check"' in builder


def test_complete_builder_preserves_public_mirror_from_public_bytes() -> None:
    builder = (PROJECT_ROOT / "scripts/build-complete-release-channel.py").read_text()

    assert "is_public_mirror" in builder
    assert 'command.extend(["--public-base", args.release_site])' in builder


def test_an_unpublished_channel_is_assembled_without_rather_than_required(monkeypatch) -> None:
    """A stable release must not depend on nightly having published.

    `release.capsem.org` answers an unpublished path with its own HTML page
    under a 200, so "no such channel" arrives as a JSON parse error rather than
    a 404. Treating that as fatal made a stable release require nightly, while
    nightly cannot publish until a stable release creates the package cohort it
    bootstraps from -- a cycle neither channel could leave, and the one that
    blocked a binary release four jobs past where any attempt had reached.
    """
    module = _module()

    def fake_read(source: str):
        if source == "source.json":
            return _graph("stable")
        raise ValueError("public preserved manifest is not a release graph")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, documents = module.resolve_channel_sources(
        explicit={"stable": "source.json"},
        primary_channel="stable",
        release_site="https://release.example",
        allow_mirror_missing=False,
    )

    assert sources == {"stable": "source.json"}, "nothing to preserve means preserve nothing"
    assert "nightly" not in documents


def test_the_cycle_is_broken_in_both_directions(monkeypatch) -> None:
    """Nightly must not depend on stable either; the list holds both."""
    module = _module()

    def fake_read(source: str):
        if source == "nightly.json":
            return _graph("nightly")
        raise ValueError("public preserved manifest is not a release graph")

    monkeypatch.setattr(module, "read_json_source", fake_read)
    sources, _ = module.resolve_channel_sources(
        explicit={"nightly": "nightly.json"},
        primary_channel="nightly",
        release_site="https://release.example",
        allow_mirror_missing=False,
    )

    assert sources == {"nightly": "nightly.json"}


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
    source = (
        PROJECT_ROOT / "scripts" / "build-complete-release-channel.py"
    ).read_text(encoding="utf-8")

    assert source.count("channels_to_assemble(") == 2, (
        "one definition and one call: a second inline list is how the build and "
        "the check disagreed about nightly"
    )
    assert source.count("for channel in build_order:") == 2, (
        "the build loop and the check loop must both iterate the resolved list"
    )
