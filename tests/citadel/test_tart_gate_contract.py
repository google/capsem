"""Tart work must be visible to the same VM scheduler as physical Apple VZ."""

from dataclasses import replace

import pytest
from capsem_builder.gate import config, host
from capsem_builder.gate.execution import Kind, Needs
from helpers.gate import PROJECT_ROOT, gate_plan

TART_RATIONALE = (
    "Tart readiness boots a real VM and macOS glow-up installs a package and "
    "boots physical Apple VZ. Both must declare VM work and claim apple_vz; "
    "otherwise scheduling and capability guards silently treat VM work as "
    "a static check. See skills/dev-gate/SKILL.md."
)


def _declares_vm_work(step) -> bool:
    claim = config.load(PROJECT_ROOT).exclusive("apple_vz")
    return step.kind is Kind.E2E and Needs.VM in step.needs and claim in step.contends


@pytest.fixture
def macos_plan(monkeypatch):
    monkeypatch.setattr(host, "system", lambda: "Darwin")
    monkeypatch.setattr(host, "machine", lambda: "arm64")
    return gate_plan("candidate")


@pytest.mark.parametrize("label", ["prepare.tart-readiness", "glowup.macos-package"])
def test_tart_steps_declare_real_vm_work(macos_plan, label):
    assert _declares_vm_work(macos_plan.step_named(label)), TART_RATIONALE


def test_vm_guard_rejects_hidden_capability_and_unclaimed_vm(macos_plan):
    step = macos_plan.step_named("glowup.macos-package")
    valid = replace(step, kind=Kind.E2E, needs=step.needs | {Needs.VM})
    assert _declares_vm_work(valid), TART_RATIONALE
    for invalid in (
        replace(valid, kind=Kind.STATIC_TEST),
        replace(valid, needs=valid.needs - {Needs.VM}),
        replace(valid, contends=()),
    ):
        assert not _declares_vm_work(invalid), TART_RATIONALE
