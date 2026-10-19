"""Cold package tests need their scratch parent before pytest creates its base."""

import os
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.gate import sdkchecks
from capsem_builder.gate.config import load
from capsem_builder.gate.context import Context
from capsem_builder.gate.fileactions import MakeDir
from capsem_builder.gate.plan import Plan
from capsem_builder.gate.proc import Runner


@pytest.mark.parametrize("adapter", [False, True])
def test_package_test_prepares_a_cold_scratch_parent(tmp_path: Path, adapter: bool) -> None:
    config = load(Path(__file__).resolve().parents[3]).model_copy(update={"root": tmp_path})
    plan = Plan("cold client scratch")
    if adapter:
        sdkchecks.inspect_fragment(plan, config, after=())
        settings, label = config.integrations_inspect_ai, "fast.integrations.inspect-ai.tests"
    else:
        sdkchecks.fragment(plan, config, after=())
        settings, label = config.sdk_python, "fast.sdk.python.tests"
    scratch = config.path(settings.pytest_scratch)
    assert not scratch.parent.exists()
    action = plan.step_named(label).actions[0]
    assert isinstance(action, MakeDir)
    action.perform(Context(Runner(tmp_path), config))
    assert scratch.parent.is_dir()
    probe = tmp_path / "test_scratch_probe.py"
    probe.write_text("def test_scratch(tmp_path):\n    assert tmp_path.is_dir()\n")
    environment = {key: value for key, value in os.environ.items()
                   if key not in {"PYTEST_ADDOPTS", "PYTEST_PLUGINS", "PYTEST_CURRENT_TEST"}}
    result = subprocess.run([sys.executable, "-m", "pytest", "-q", "-c", "/dev/null", "--basetemp", str(scratch), str(probe)],
                            cwd=tmp_path, env=environment, check=False, capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stdout + result.stderr
