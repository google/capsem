"""Release inputs must remain data when GitHub renders shell commands."""

from pathlib import Path

import pytest
from helpers.workflow_contract import workflow_jobs

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("workflow", ["release-runtime-preflight.yaml", "release-binary-staging.yaml", "ci.yaml"])
def test_release_inputs_enter_shell_only_through_environment(workflow: str) -> None:
    for job in workflow_jobs(ROOT / ".github/workflows" / workflow).values():
        for step in job.get("steps", []):
            assert not any(token in step.get("run", "") for token in ("${{ inputs.", "${{ steps.")), (
                f"{workflow}: {step.get('name')}: template-expanded input becomes executable shell; "
                "pass it through env and quote its shell variable instead"
            )


@pytest.mark.parametrize("attack_kind", ["quote", "substitution"])
def test_native_candidate_output_is_one_literal_shell_argument(tmp_path, attack_kind) -> None:
    import json
    import os
    import shlex
    import subprocess
    import sys

    step = next(step for step in workflow_jobs(ROOT / ".github/workflows/ci.yaml")["test-install"]["steps"]
                if step.get("name") == "Qualify exact native capsem-debug candidate")
    marker = tmp_path / "injected-command-ran"
    attack = (f'"; touch {shlex.quote(str(marker))}; #' if attack_kind == "quote"
              else f'$(touch {shlex.quote(str(marker))})')
    executable = tmp_path / "uv"
    executable.write_text(f"#!{sys.executable}\nimport json, os, sys\n"
                          "with open(os.environ['ARGV_FILE'], 'w') as output: json.dump(sys.argv[1:], output)\n")
    executable.chmod(0o755)
    arguments = tmp_path / "argv.json"
    # Render the old vulnerable shape too: this test exercises Bash parsing,
    # while only the unrelated image qualification executable is replaced.
    body = step["run"].replace("${{ steps.select_debug_candidate.outputs.layout }}", attack)
    subprocess.run(["bash", "-eu", "-c", body], cwd=tmp_path, timeout=10, check=True,
                   env={"PATH": f"{tmp_path}{os.pathsep}{os.defpath}", "ARGV_FILE": str(arguments),
                        "NATIVE_CANDIDATE_LAYOUT": attack})
    assert not marker.exists(), "CI output must enter Bash as data, never executable source"
    argv = json.loads(arguments.read_text())
    assert argv[argv.index("--layout") + 1] == attack
    assert argv[-1] == "--native-report"
