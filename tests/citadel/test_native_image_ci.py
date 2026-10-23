"""Native candidate transfer must preserve the existing read-only install gate."""

import shlex
import tomllib
from copy import deepcopy
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = "Native image artifacts require both real qualifications; CI must not acquire publishing authority."


def validate(workflow, settings):
    assert (settings["platform"], settings["system"], settings["machine"]) == (
        "linux/amd64",
        "Linux",
        "x86_64",
    ), RATIONALE
    assert settings["module"] == "tests.fixtures.oci.native_handoff", RATIONALE
    assert workflow["permissions"] == {"contents": "read", "packages": "read"}, RATIONALE
    job = workflow["jobs"]["test-install"]
    assert job["runs-on"] == "ubuntu-24.04" and job["timeout-minutes"] == 240, RATIONALE
    assert "permissions" not in job, RATIONALE
    steps = job["steps"]
    indexed = {step.get("id"): step for step in steps if step.get("id")}
    expected = {
        "build_debug_candidate": ["capsem-gate", "debug-image", "--candidate-receipt"],
        "select_debug_candidate": ["python", "-m", settings["module"], "select", "debug_image"],
        "qualify_debug_candidate": [
            "capsem-gate",
            "image-qualify",
            "--image",
            "capsem-debug",
            "--layout",
            "${{ steps.select_debug_candidate.outputs.layout }}",
            "--native-report",
        ],
        "build_dev_candidate": ["capsem-gate", "reference-image", "--candidate-receipt"],
        "select_dev_candidate": ["python", "-m", settings["module"], "select", "reference_image"],
        "qualify_dev_candidate": [
            "capsem-gate",
            "image-qualify",
            "--image",
            "dev",
            "--layout",
            "${{ steps.select_dev_candidate.outputs.layout }}",
            "--native-report",
        ],
        "stage_native_candidates": ["python", "-m", settings["module"], "stage"],
    }
    positions = []
    for key, command in expected.items():
        step = indexed[key]
        assert not step.get("continue-on-error") and not step.get("if"), RATIONALE
        assert shlex.split(step["run"]) == [
            "uv",
            "run",
            "--project",
            "build_system",
            "--frozen",
            *command,
        ], RATIONALE
        positions.append(steps.index(step))
    assert positions == sorted(positions), RATIONALE
    originals = [
        step
        for step in steps
        if step.get("name") in ("Run install e2e tests", "Prove live sibling VM isolation")
    ]
    assert len(originals) == 2 and all(steps.index(step) < positions[0] for step in originals), (
        RATIONALE
    )
    upload = next(step for step in steps if step.get("id") == "upload_native_candidates")
    assert steps.index(upload) > positions[-1] and upload.get("if") == "${{ success() }}", RATIONALE
    assert upload["uses"] == "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", (
        RATIONALE
    )
    options = upload["with"]
    assert (
        options["path"] == settings["artifact_dir"] and options["if-no-files-found"] == "error"
    ), RATIONALE
    assert options["retention-days"] == 7 and not options.get("overwrite"), RATIONALE
    assert (
        options["name"]
        == "native-image-candidates-amd64-${{ github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}"
    ), RATIONALE


def inputs():
    workflow = yaml.safe_load((ROOT / ".github/workflows/ci.yaml").read_text())
    settings = tomllib.loads((ROOT / "config/gate.toml").read_text())["functional"]["native_images"]
    return workflow, settings


def test_existing_native_job_keeps_fixed_qualification_and_nonpublishing_transfer():
    validate(*inputs())


@pytest.mark.parametrize(
    "defect",
    [
        "permission",
        "timeout",
        "runner",
        "skip",
        "mask",
        "wrong-image",
        "order",
        "upload-failure",
        "upload-tree",
        "upload-overwrite",
        "missing-qualifier",
    ],
)
def test_guard_rejects_authority_or_proof_evasions(defect):
    original, settings = inputs()
    workflow = deepcopy(original)
    job = workflow["jobs"]["test-install"]
    steps = job["steps"]
    indexed = {step.get("id"): step for step in steps if step.get("id")}
    if defect == "permission":
        workflow["permissions"]["packages"] = "write"
    if defect == "timeout":
        job["timeout-minutes"] = 300
    if defect == "runner":
        job["runs-on"] = "ubuntu-24.04-arm"
    if defect == "skip":
        indexed["qualify_debug_candidate"]["if"] = "false"
    if defect == "mask":
        indexed["qualify_debug_candidate"]["continue-on-error"] = True
    if defect == "wrong-image":
        indexed["qualify_debug_candidate"]["run"] = indexed["qualify_debug_candidate"][
            "run"
        ].replace("capsem-debug", "dev")
    if defect == "order":
        steps.remove(indexed["stage_native_candidates"])
        steps.insert(0, indexed["stage_native_candidates"])
    if defect == "upload-failure":
        indexed["upload_native_candidates"]["if"] = "always()"
    if defect == "upload-tree":
        indexed["upload_native_candidates"]["with"]["path"] = "cache/"
    if defect == "upload-overwrite":
        indexed["upload_native_candidates"]["with"]["overwrite"] = True
    if defect == "missing-qualifier":
        steps.remove(indexed["qualify_dev_candidate"])
    with pytest.raises((AssertionError, KeyError)):
        validate(workflow, settings)
