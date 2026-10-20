"""Release inputs must remain data when GitHub renders shell commands."""

from pathlib import Path

import pytest
from helpers.workflow_contract import workflow_jobs

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("workflow", ["release-runtime-preflight.yaml", "release-binary-staging.yaml"])
def test_release_inputs_enter_shell_only_through_environment(workflow: str) -> None:
    for job in workflow_jobs(ROOT / ".github/workflows" / workflow).values():
        for step in job.get("steps", []):
            assert "${{ inputs." not in step.get("run", ""), (
                f"{workflow}: {step.get('name')}: template-expanded input becomes executable shell; "
                "pass it through env and quote its shell variable instead"
            )
