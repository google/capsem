"""The stopped-workspace fixture seeds only an isolated registry."""

from __future__ import annotations

import json
import runpy
import stat
from pathlib import Path

import pytest
from helpers import stopped_workspace
from helpers.stopped_workspace import seed_stopped_workspace


@pytest.mark.parametrize("architecture", ["arm64", "x86_64"])
@pytest.mark.parametrize("release_graph", [False, True])
def test_fixture_pins_the_selected_manifest_architecture(tmp_path, monkeypatch, architecture, release_graph):
    architectures = {}
    for offset, arch in enumerate(("arm64", "x86_64")):
        architectures[arch] = {
            name: {"hash": f"{offset * 10 + index + 1:064x}"}
            for index, name in enumerate(stopped_workspace.BOOT_ASSETS.values())
        }
    manifest = {"assets": {"current": "selected", "releases": {"selected": {"arches": architectures}}}}
    if release_graph:
        manifest = {"runtime": {"architectures": [
            {"architecture": arch, "images": [
                {"kind": kind, "bytes": 1, "digest": {"sha256": "f" * 64, "blake3": entries[name]["hash"]}}
                for kind, name in stopped_workspace.BOOT_ASSETS.items()
            ]}
            for arch, entries in architectures.items()
        ]}}
    (tmp_path / "manifest.json").write_text(json.dumps(manifest))
    monkeypatch.setattr(stopped_workspace.platform, "machine", lambda: architecture)
    assert stopped_workspace.manifest_asset_pins(tmp_path) == {
        kind: {"name": name, "hash": "blake3:" + architectures[architecture][name]["hash"]}
        for kind, name in stopped_workspace.BOOT_ASSETS.items()
    }
    integrity = runpy.run_path(str(Path(__file__).parent / "capsem-security/test_asset_integrity.py"))
    resolver = integrity["_arch_manifest"]
    monkeypatch.setitem(resolver.__globals__, "ASSETS_DIR", tmp_path)
    try:
        entries = resolver(architecture)
    except pytest.skip.Exception as error:
        pytest.fail(f"selected runtime integrity must be checked: {error}")
    assert entries == architectures[architecture]


@pytest.mark.parametrize("runtime", [
    {"status": "revoked"},
    {"architectures": []},
    {"architectures": [{"architecture": "x86_64", "images": []}]},
    {"architectures": [{"architecture": "x86_64"}, {"architecture": "x86_64"}]},
])
def test_selected_runtime_cannot_fall_back_to_other_local_assets(tmp_path, monkeypatch, runtime):
    local = {name: {"hash": "a" * 64} for name in stopped_workspace.BOOT_ASSETS.values()}
    manifest = {
        "assets": {"current": "other", "releases": {"other": {"arches": {"x86_64": local}}}},
        "runtime": runtime,
    }
    (tmp_path / "manifest.json").write_text(json.dumps(manifest))
    monkeypatch.setattr(stopped_workspace.platform, "machine", lambda: "x86_64")
    with pytest.raises(ValueError):
        stopped_workspace.manifest_asset_pins(tmp_path)


def test_fixture_never_replaces_an_existing_registry(tmp_path: Path) -> None:
    registry = tmp_path / "persistent_registry.json"
    registry.write_text("existing registry")
    with pytest.raises(ValueError, match="isolated empty registry"):
        seed_stopped_workspace(tmp_path, tmp_path / "assets")
    assert registry.read_text() == "existing registry"
    assert not (tmp_path / "persistent").exists()


def test_fixture_makes_ledger_files_private(tmp_path: Path, monkeypatch) -> None:
    real_copy2 = stopped_workspace.shutil.copy2
    real_copytree = stopped_workspace.shutil.copytree

    def permissive_copy2(source: Path, destination: Path) -> Path:
        copied = Path(real_copy2(source, destination))
        copied.chmod(0o644)
        return copied

    def permissive_copytree(source: Path, destination: Path) -> Path:
        copied = real_copytree(source, destination)
        copied.chmod(0o755)
        return copied

    monkeypatch.setattr(stopped_workspace, "manifest_asset_pins", lambda _assets: {})
    monkeypatch.setattr(stopped_workspace.shutil, "copy2", permissive_copy2)
    monkeypatch.setattr(stopped_workspace.shutil, "copytree", permissive_copytree)

    seed_stopped_workspace(tmp_path, tmp_path / "assets")

    mode = stat.S_IMODE(
        (tmp_path / "persistent" / stopped_workspace.VM_ID / "session.bodies")
        .stat()
        .st_mode
    )
    assert mode == 0o700
    session = tmp_path / "persistent" / stopped_workspace.VM_ID
    assert stat.S_IMODE((session / "session.db").stat().st_mode) == 0o600
    assert stat.S_IMODE((session / "session.db-archive.lock").stat().st_mode) == 0o600
    assert {
        stat.S_IMODE(path.stat().st_mode)
        for path in (session / "session.bodies").rglob("*")
        if path.is_file()
    } == {0o600}
