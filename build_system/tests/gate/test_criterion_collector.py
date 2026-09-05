"""Criterion cases must reach the recorded metrics, including ungrouped cases."""

import json
import runpy
from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config


@pytest.mark.parametrize("case", ["security_registry_builtin_clone", "group/case", "security_actions/case"])
def test_target_output_includes_every_case(tmp_path, monkeypatch, case):
    root = Path(__file__).resolve().parents[3]
    settings = gate_config.load(root).benchmark.run
    collector = runpy.run_path(str(root / settings.collectors / "criterion"))
    run_bench = collector["run_bench"]
    run_bench.__globals__["CRITERION"] = tmp_path
    collector["samples_for"].__globals__["CRITERION"] = tmp_path

    def run(command, **kwargs):
        # Criterion defaults to a shared output root, not the Cargo target name.
        output = Path(kwargs.get("env", {}).get("CRITERION_HOME", tmp_path))
        sample = output / case / "new" / "sample.json"
        sample.parent.mkdir(parents=True)
        sample.write_text(json.dumps({"times": [100, 300], "iters": [10, 20]}))

    monkeypatch.setattr(run_bench.__globals__["subprocess"], "run", run)
    run_bench("capsem-core", "security_actions", sample_size=None)
    assert collector["samples_for"]("security_actions") == {
        case.removeprefix("security_actions/"): [10.0, 15.0]
    }
