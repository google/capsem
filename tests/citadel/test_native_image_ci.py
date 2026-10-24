"""Six native AMD subjects use the same read-only rail and retained transfer owner."""

import shlex
import tomllib
from copy import deepcopy
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = "Native image artifacts require original qualifications and held leases through official upload/download; CI has no publishing authority."
UV = "uv run --project build_system --frozen "
MODULE = "python -m tests.fixtures.oci.native_handoff"
WRAPPER = UV + "python build_system/scripts/ci/run-bounded-command.py --timeout-seconds "
IDENTITY = ' --run-id "$GITHUB_RUN_ID" --attempt "$GITHUB_RUN_ATTEMPT"'
UPLOAD = "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
DOWNLOAD = "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c"
CANDIDATES = (
    ("debug", "debug_image", "capsem-debug"),
    ("dev", "reference_image", "dev"),
    ("codex_cli", "codex-cli", "codex-cli"),
    ("claude_code", "claude-code", "claude-code"),
    ("agy", "agy", "agy"),
    ("antigravity_desktop", "antigravity-desktop", "antigravity-desktop"),
)


def expression(field):
    return "${{ steps.native_transfer_limits.outputs." + field + " }}"


def invocation(action, section, budget):
    return (
        WRAPPER
        + '"'
        + expression(budget)
        + '" -- '
        + UV
        + MODULE
        + " "
        + action
        + " "
        + section
        + IDENTITY
    )


def held(step, section, *, base=False):
    assert not step.get("if") and not step.get("continue-on-error"), RATIONALE
    assert step["run"].strip().splitlines() == [
        invocation("hold", section, "base_outer_seconds" if base else "outer_seconds")
        + ' > "'
        + expression("control_dir")
        + "/"
        + section
        + '-owner.log" 2>&1 &',
        invocation("ready", section, "ready_seconds"),
    ], RATIONALE


def validate(workflow, settings):
    assert (settings["platform"], settings["system"], settings["machine"]) == (
        "linux/amd64",
        "Linux",
        "x86_64",
    ), RATIONALE
    assert settings["module"] == "tests.fixtures.oci.native_handoff", RATIONALE
    assert set(settings["apps"]) == {image for _, _, image in CANDIDATES[2:]}, RATIONALE
    assert workflow["permissions"] == {"contents": "read", "packages": "read"}, RATIONALE
    job = workflow["jobs"]["test-install"]
    assert job["runs-on"] == "ubuntu-24.04" and job["timeout-minutes"] == 240, RATIONALE
    assert "permissions" not in job and "strategy" not in job, RATIONALE
    assert settings["transfer"]["base_outer_seconds"] <= job["timeout-minutes"] * 60, RATIONALE
    steps = job["steps"]
    indexed = {step.get("id"): step for step in steps if step.get("id")}
    configure = indexed["native_transfer_limits"]
    assert configure["run"] == UV + MODULE + " configure", RATIONALE
    assert not configure.get("if") and not configure.get("continue-on-error"), RATIONALE
    originals = [
        step
        for step in steps
        if step.get("name") in ("Run install e2e tests", "Prove live sibling VM isolation")
    ]
    assert len(originals) == 2 and all(
        steps.index(step) < steps.index(configure) for step in originals
    ), RATIONALE
    positions = [steps.index(configure)]
    for label, section, image in CANDIDATES:
        expected = {
            "build": [
                "capsem-gate",
                "debug-image" if section == "debug_image" else "reference-image",
                "--candidate-receipt",
            ]
            + ([] if section == "debug_image" else ["--candidate-image", image]),
            "select": [*MODULE.split(), "select", section],
            "qualify": [
                "capsem-gate",
                "image-qualify",
                "--image",
                image,
                "--layout",
                "${{ steps.select_" + label + "_candidate.outputs.layout }}",
                "--native-report",
            ],
        }
        for operation, command in expected.items():
            step = indexed[operation + "_" + label + "_candidate"]
            assert not step.get("if") and not step.get("continue-on-error"), RATIONALE
            assert shlex.split(step["run"]) == [*UV.split(), *command], RATIONALE
            positions.append(steps.index(step))
        if image == "dev":
            held(indexed["retain_native_base"], "base", base=True)
            assert (
                steps.index(indexed["build_dev_candidate"])
                < steps.index(indexed["retain_native_base"])
                < steps.index(indexed["select_dev_candidate"])
            ), RATIONALE
        retain = indexed["retain_" + label + "_candidate"]
        held(retain, section)
        positions.append(steps.index(retain))
        upload = indexed["upload_" + label + "_candidate"]
        assert (
            upload["uses"] == UPLOAD
            and upload.get("if") == "${{ success() }}"
            and not upload.get("continue-on-error")
        ), RATIONALE
        assert (
            upload["timeout-minutes"]
            == "${{ fromJSON(steps.native_transfer_limits.outputs.upload_minutes) }}"
        ), RATIONALE
        assert upload["with"] == {
            "name": "${{ steps.retain_" + label + "_candidate.outputs.artifact_name }}",
            "path": "${{ steps.retain_" + label + "_candidate.outputs.artifact_path }}",
            "retention-days": 7,
            "if-no-files-found": "error",
            "compression-level": 0,
        }, RATIONALE
        download = indexed["download_" + label + "_candidate"]
        assert (
            download["uses"] == DOWNLOAD
            and download.get("if") == "${{ success() }}"
            and not download.get("continue-on-error")
        ), RATIONALE
        assert (
            download["timeout-minutes"]
            == "${{ fromJSON(steps.native_transfer_limits.outputs.download_minutes) }}"
        ), RATIONALE
        assert download["with"] == {
            "artifact-ids": "${{ steps.upload_" + label + "_candidate.outputs.artifact-id }}",
            "path": "${{ steps.retain_" + label + "_candidate.outputs.download_path }}",
            "merge-multiple": True,
            "digest-mismatch": "error",
        }, RATIONALE
        release = indexed["release_" + label + "_candidate"]
        assert (
            release["run"]
            == invocation("release", section, "settle_seconds")
            + ' --artifact-id "${{ steps.upload_'
            + label
            + '_candidate.outputs.artifact-id }}"'
        ), RATIONALE
        assert not release.get("if") and not release.get("continue-on-error"), RATIONALE
        abort = indexed["abort_" + label + "_candidate"]
        assert abort["run"] == invocation("abort", section, "settle_seconds"), RATIONALE
        assert (
            abort.get("if")
            == "${{ always() && steps.retain_"
            + label
            + "_candidate.outcome != 'skipped' && steps.release_"
            + label
            + "_candidate.outcome != 'success' }}"
        ), RATIONALE
        assert not abort.get("continue-on-error"), RATIONALE
        positions += [steps.index(step) for step in (upload, download, release, abort)]
    assert positions == sorted(set(positions)), RATIONALE
    release_base = indexed["release_native_base"]
    assert release_base["run"] == invocation(
        "release", "base", "settle_seconds"
    ) and not release_base.get("if"), RATIONALE
    finish = indexed["finish_native_candidates"]
    assert finish["run"] == UV + MODULE + " finish" + IDENTITY and not finish.get("if"), RATIONALE
    assert positions[-1] < steps.index(release_base) < steps.index(finish), RATIONALE
    upload = indexed["upload_native_candidates"]
    assert (
        upload["uses"] == UPLOAD
        and upload.get("if") == "${{ success() }}"
        and not upload.get("continue-on-error")
    ), RATIONALE
    assert (
        upload["with"]["path"] == "${{ steps.finish_native_candidates.outputs.manifest_path }}"
    ), RATIONALE
    assert (
        upload["with"]["name"]
        == "native-image-candidates-amd64-${{ github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}"
    ), RATIONALE
    assert (
        upload["with"]["retention-days"] == 7
        and upload["with"]["if-no-files-found"] == "error"
        and not upload["with"].get("overwrite")
    ), RATIONALE
    assert steps.index(finish) < steps.index(upload), RATIONALE
    abort_base = indexed["abort_native_base"]
    assert abort_base["run"] == invocation("abort", "base", "settle_seconds"), RATIONALE
    assert (
        abort_base.get("if")
        == "${{ always() && steps.retain_native_base.outcome != 'skipped' && steps.release_native_base.outcome != 'success' }}"
    ), RATIONALE


def inputs():
    return yaml.safe_load((ROOT / ".github/workflows/ci.yaml").read_text()), tomllib.loads(
        (ROOT / "config/gate.toml").read_text()
    )["functional"]["native_images"]


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
        "missing-download",
        "all-held",
        "early-release",
        "wrong-download",
        "no-shared-base",
        "foreign-artifact",
        "ignored-abort",
    ],
)
def test_guard_rejects_authority_proof_or_lifetime_evasions(defect):
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
    if defect == "early-release":
        step = indexed["release_debug_candidate"]
        steps.remove(step)
        steps.insert(0, step)
    if defect == "order":
        step = indexed["build_dev_candidate"]
        steps.remove(step)
        steps.insert(steps.index(indexed["upload_debug_candidate"]), step)
    if defect == "upload-failure":
        indexed["upload_debug_candidate"]["if"] = "always()"
    if defect == "upload-tree":
        indexed["upload_debug_candidate"]["with"]["path"] = "cache/"
    if defect == "all-held":
        indexed["retain_debug_candidate"]["run"] = UV + MODULE + " stage"
    if defect == "upload-overwrite":
        indexed["upload_debug_candidate"]["with"]["overwrite"] = True
    if defect == "missing-qualifier":
        steps.remove(indexed["qualify_antigravity_desktop_candidate"])
    if defect == "missing-download":
        steps.remove(indexed["download_agy_candidate"])
    if defect == "wrong-download":
        indexed["download_agy_candidate"]["with"]["artifact-ids"] = "arbitrary"
    if defect == "no-shared-base":
        steps.remove(indexed["retain_native_base"])
    if defect == "foreign-artifact":
        indexed["release_dev_candidate"]["run"] += " --artifact-id 1"
    if defect == "ignored-abort":
        indexed["abort_dev_candidate"]["continue-on-error"] = True
    with pytest.raises((AssertionError, KeyError)):
        validate(workflow, settings)
