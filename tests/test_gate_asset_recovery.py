"""The warm asset shortcut may change work, never the candidate graph."""

from __future__ import annotations

import json
from pathlib import Path

import blake3
from helpers.gate import RecordingJournal, RecordingRunner

from capsem.gate import config as gate_config
from capsem.gate import imagebuild
from capsem.gate.actions import Action
from capsem.gate.assetcondition import AssetRecovery
from capsem.gate.context import Context
from capsem.gate.plan import Plan

PROJECT_ROOT = Path(__file__).resolve().parents[1]


class _Probe(Action, name="asset-recovery-probe"):
    def __init__(self, calls: list[str]) -> None:
        self._calls = calls

    def render(self) -> str:
        return "recover host assets"

    def perform(self, context: Context) -> None:
        del context
        self._calls.append("performed")


def _obom_payload(arch: str) -> bytes:
    return json.dumps(
        {
            "bomFormat": "CycloneDX",
            "metadata": {
                "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                "component": {
                    "type": "operating-system",
                    "name": f"capsem-rootfs-{arch}",
                    "version": "guest-rootfs",
                    "properties": [
                        {"name": "capsem:evidence:scope", "value": "exported-rootfs"},
                        {"name": "capsem:guest:architecture", "value": arch},
                    ],
                },
            },
            "components": [{"purl": "pkg:deb/debian/base-files@1"}],
        }
    ).encode()


def _seed_assets(config) -> Path:
    tree = config.path(config.imagebuild.output) / config.host_arch().name
    tree.mkdir(parents=True)
    required = (*config.artifacts.bootable, *config.assets.evidence_artifacts)
    entries = {}
    for name in required:
        payload = (
            _obom_payload(config.host_arch().name)
            if name == config.assets.obom_artifact
            else b"qualified"
        )
        (tree / name).write_bytes(payload)
        entries[name] = {
            "hash": blake3.blake3(payload).hexdigest(),
            "sha256": "0" * 64,
            "size": len(payload),
        }
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "assets": {
                    "current": "test",
                    "releases": {"test": {"arches": {config.host_arch().name: entries}}},
                },
            }
        ),
        encoding="utf-8",
    )
    return manifest


def _recovery_plan(config, monkeypatch) -> Plan:
    monkeypatch.setattr(imagebuild, "profiles", lambda _config: ["code"])
    plan = Plan("asset-recovery")
    imagebuild.check_assets(plan, config)
    return plan


def test_warm_and_cold_checkouts_describe_the_same_recovery_graph(
    monkeypatch,
) -> None:
    config = gate_config.load(PROJECT_ROOT)
    monkeypatch.setattr(imagebuild, "missing", lambda *_args: [])
    warm_plan = _recovery_plan(config, monkeypatch)

    monkeypatch.setattr(imagebuild, "missing", lambda *_args: ["initrd.img"])
    cold_plan = _recovery_plan(config, monkeypatch)

    assert warm_plan.labels == cold_plan.labels
    assert warm_plan.edges == cold_plan.edges
    assert "assets.asset-tools" in warm_plan.labels


def test_describing_recovery_never_reads_asset_presence(monkeypatch) -> None:
    config = gate_config.load(PROJECT_ROOT)

    def refuse_plan_time_read(*_args, **_kwargs):
        raise AssertionError("asset presence was read while describing the plan")

    monkeypatch.setattr("capsem.gate.assetcondition.missing", refuse_plan_time_read)

    plan = _recovery_plan(config, monkeypatch)

    assert "assets.asset-tools" in plan.labels


def test_recovery_action_skips_warm_assets_and_runs_for_cold_assets(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    calls: list[str] = []
    recovery = AssetRecovery(config, arch)
    action = recovery.when(_Probe(calls))
    context = Context(
        RecordingRunner(tmp_path),
        config,
        journal=RecordingJournal(),
    )
    _seed_assets(config)

    action.perform(context)
    assert calls == []

    missing_asset = config.path(config.imagebuild.output) / arch.name / config.artifacts.bootable[0]
    missing_asset.unlink()
    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]
    assert action.render() == "when host assets are missing: recover host assets"


def test_nonempty_partial_asset_output_cannot_satisfy_recovery(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    tree = config.path(config.imagebuild.output) / arch.name
    tree.mkdir(parents=True)
    for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
        (tree / name).write_bytes(b"partial producer output")
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]


def test_manifest_digest_must_match_every_boot_and_evidence_artifact(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    _seed_assets(config)
    changed = config.path(config.imagebuild.output) / arch.name / config.assets.obom_artifact
    changed.write_bytes(b"nonempty but not the completed producer output")
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]


def test_rehashed_raw_scanner_output_is_not_a_completed_asset_build(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    arch = config.host_arch()
    manifest = _seed_assets(config)
    obom = config.path(config.imagebuild.output) / arch.name / config.assets.obom_artifact
    raw = json.dumps(
        {
            "bomFormat": "CycloneDX",
            "metadata": {
                "timestamp": "2026-08-12T00:00:00Z",
                "tools": {"components": [{"name": "cdxgen", "version": "12.7.0"}]},
                "component": {"type": "container", "name": "rootfs"},
            },
            "components": [{"purl": "pkg:deb/debian/base-files@1"}],
        }
    ).encode()
    obom.write_bytes(raw)
    document = json.loads(manifest.read_text(encoding="utf-8"))
    entry = document["assets"]["releases"]["test"]["arches"][arch.name][obom.name]
    entry["hash"] = blake3.blake3(raw).hexdigest()
    entry["size"] = len(raw)
    manifest.write_text(json.dumps(document), encoding="utf-8")
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    AssetRecovery(config, arch).when(_Probe(calls)).perform(context)

    assert calls == ["performed"]


def test_one_cold_decision_covers_every_action_in_the_recovery(tmp_path: Path) -> None:
    base = gate_config.load(PROJECT_ROOT)
    config = base.model_copy(update={"root": tmp_path})
    recovery = AssetRecovery(config, config.host_arch())
    calls: list[str] = []
    context = Context(RecordingRunner(tmp_path), config, journal=RecordingJournal())

    recovery.when(_Probe(calls)).perform(context)
    _seed_assets(config)
    recovery.when(_Probe(calls)).perform(context)

    assert calls == ["performed", "performed"]


def test_profile_recovery_builds_are_ordered_and_invalidate_completion_first(
    monkeypatch,
) -> None:
    config = gate_config.load(PROJECT_ROOT)
    monkeypatch.setattr(imagebuild, "profiles", lambda _config: ["code", "co-work"])

    plan = Plan("asset-recovery-order")
    images = imagebuild.check_assets(plan, config)

    first, second = images
    assert (first.label, second.label) in plan.edges
    manifest = config.path(config.imagebuild.output) / config.install.manifest_name
    assert first.actions[0].render() == f"when host assets are missing: rm -rf {manifest}"
    assert "capsem-admin" in first.actions[1].render()
