"""Install CI must keep native restart failures alongside installed proof logs."""

from pathlib import Path, PurePosixPath

import pytest
import yaml
from capsem_builder.gate.config import load

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = (
    "The install job runs managed SDK restart before package installation. "
    "Uploading only the installed/ child discarded both hosted startup failures; "
    "preserve the complete configured test-evidence root."
)


def require_evidence_root(selection: str, evidence: str) -> None:
    selected = {PurePosixPath(line.strip()) for line in selection.splitlines() if line.strip()}
    assert PurePosixPath(evidence) in selected, RATIONALE


def test_install_job_preserves_every_failed_fixture() -> None:
    document = yaml.safe_load((ROOT / ".github/workflows/ci.yaml").read_text())
    steps = document["jobs"]["test-install"]["steps"]
    upload = next(step for step in steps if step.get("name") == "Upload install and glow-up evidence on failure")
    assert upload["if"] == "failure()", RATIONALE
    require_evidence_root(upload["with"]["path"], load(ROOT).outputs.test_artifacts)


def test_installed_subtree_alone_cannot_cover_native_restart_evidence() -> None:
    evidence = "proof/evidence"
    require_evidence_root(evidence + "/", evidence)
    with pytest.raises(AssertionError, match="managed SDK restart"):
        require_evidence_root(evidence + "/installed/", evidence)
