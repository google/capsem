"""The fixed existing producers/qualifier receive opt-in evidence flags only."""

from argparse import ArgumentParser, Namespace
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.debugimage import DebugImageCommand, ReferenceImageCommand, _build
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.imagequalify import ImageQualifyModule
from capsem_builder.gate.proc import Runner

ROOT = Path(__file__).resolve().parents[3]
CONFIG = gate_config.load(ROOT)


@pytest.mark.parametrize(
    "command,section",
    [(DebugImageCommand, "debug_image"), (ReferenceImageCommand, "reference_image")],
)
def test_opt_in_receipt_uses_the_existing_producer_without_nested_gate(command, section):
    parser = ArgumentParser()
    command.add_arguments(parser)
    assert parser.parse_args(["--candidate-receipt"]).candidate_receipt is True
    plan = _build(command.name, CONFIG, section, receipt=True)
    (step,) = plan.steps
    rendered = step.actions[0].render()
    assert getattr(CONFIG.functional, section).script in rendered
    assert "--receipt" in rendered and "capsem-gate" not in rendered


def test_original_producer_invocation_remains_receipt_free():
    plan = _build("debug-image", CONFIG, "debug_image")
    assert "--receipt" not in plan.steps[0].actions[0].render()


def test_candidate_selector_is_fixed_opt_in_on_existing_reference_owner():
    parser = ArgumentParser()
    ReferenceImageCommand.add_arguments(parser)
    args = parser.parse_args(["--candidate-receipt", "--candidate-image", "codex-cli"])
    assert args.candidate_image == "codex-cli"
    rendered = (
        _build("reference-image", CONFIG, "reference_image", receipt=True, candidate="codex-cli")
        .steps[0]
        .actions[0]
        .render()
    )
    assert "--candidate-image codex-cli" in rendered and "--receipt" in rendered
    with pytest.raises(GateError, match="receipt"):
        _build("reference-image", CONFIG, "reference_image", candidate="codex-cli")
    with pytest.raises(GateError):
        _build("debug-image", CONFIG, "debug_image", receipt=True, candidate="codex-cli")
    with pytest.raises(SystemExit):
        parser.parse_args(["--candidate-image", "claude-desktop"])


def test_native_report_requires_explicit_original_candidate():
    module = ImageQualifyModule(
        Runner(ROOT), Namespace(image=None, layout=None, native_report=True)
    )
    with pytest.raises(GateError, match="explicit candidate"):
        module.plan()
