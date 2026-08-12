"""The warm asset shortcut may change work, never the candidate graph."""

from __future__ import annotations

from pathlib import Path

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


def _seed_assets(config) -> None:
    tree = config.path(config.imagebuild.output) / config.host_arch().name
    tree.mkdir(parents=True)
    for name in config.artifacts.bootable:
        (tree / name).write_bytes(b"qualified")


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
