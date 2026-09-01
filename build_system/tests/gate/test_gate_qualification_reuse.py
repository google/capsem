"""One fresh proof per cohort inside a complete local qualification."""

from __future__ import annotations

import argparse
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate import module_qualify, qualification
from capsem_builder.gate.candidate import CandidateCommand
from capsem_builder.gate.content import ProfileContent
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.resume import ancestors
from helpers.gate import RecordingRunner

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)


def _candidate() -> Plan:
    command = CandidateCommand(
        RecordingRunner(ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
    )
    return command._describe()


def _release_pairing() -> Plan:
    plan = Plan("release-pairing")
    state = qualification.rehearsal(
        CONFIG,
        input_dir="cache/target/release-inputs",
        package="cache/target/package.deb",
    )
    return module_qualify._pairing(
        plan,
        CONFIG,
        state,
        staged=ProfileContent.staged(CONFIG, ROOT),
    )


def test_composed_gate_installs_each_node_workspace_once() -> None:
    plan = _candidate()
    owners = {
        step.label
        for step in plan.steps
        if any("pnpm install --frozen-lockfile" in line for line in step.render())
    }

    assert owners == {"fast.toolchain.node"}
    assert "contracts.toolchain.node" not in plan.labels
    assert "static.toolchain.node" not in plan.labels


def test_source_contract_coverage_is_handed_to_the_fresh_vm_cohort() -> None:
    plan = _candidate()
    seeded = " ".join(plan.step_named("contracts.release").render())
    appended = " ".join(plan.step_named("contracts.build-system").render())
    finished = " ".join(plan.step_named("functional.pytest.broad.code").render())

    assert "--cov-report=" in seeded and "--cov-append" not in seeded
    assert "--cov-append" in appended and "--cov-fail-under=0" in appended
    assert "--cov-append" in finished
    assert "--cov-report=xml:cache/target/coverage/python/codecov.xml" in finished
    for path in CONFIG.suites.source_contract:
        assert f"--ignore={path}" in finished
    for pattern in CONFIG.modules.contract_globs:
        assert f"--ignore-glob={pattern}" in finished


def test_restaged_release_path_reuses_behavior_only_after_digest_verification() -> None:
    plan = _candidate()
    labels = set(plan.labels)
    repeated = (
        "rehearsal.pytest.",
        "rehearsal.injection.",
        "rehearsal.integration.",
    )

    assert "rehearsal.release-inputs.verify" in labels
    assert "rehearsal.axis" in labels
    assert not any(label.startswith(repeated) for label in labels)
    assert "functional.pytest.broad.code" in labels
    assert "functional.pytest.compatibility.co-work" in labels
    assert "functional.pytest.benchmark.code" in labels
    prerequisites = ancestors(plan, "rehearsal.release-inputs.verify")
    assert "functional.pytest.broad.code" in prerequisites
    assert "functional.integration.co-work" in prerequisites


def test_release_pairing_remains_independently_full_and_fresh() -> None:
    plan = _release_pairing()
    labels = set(plan.labels)
    broad = " ".join(plan.step_named("functional.pytest.broad.code").render())

    assert {
        "artifacts.release-inputs.verify",
        "functional.pytest.broad.code",
        "functional.pytest.host-snapshot.code",
        "functional.pytest.timing.code",
        "functional.injection.code",
        "functional.integration.code",
        "functional.pytest.benchmark.code",
        "glowup.package",
    } <= labels
    assert "--cov-append" not in broad
    assert "--cov-report=xml:cache/target/coverage/python/codecov.xml" in broad
    for path in CONFIG.suites.source_contract:
        assert f"--ignore={path}" not in broad
