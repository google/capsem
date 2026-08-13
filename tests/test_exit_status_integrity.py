"""A gate's result must not be read from the last line of a multi-part output.

Two shapes of the same mistake, both of which report success while the thing
being measured failed:

1. `$?` after a pipe is the *pipe's* status. `just test | tail` reports what
   `tail` did. Under `set -o pipefail` the pipeline adopts the first non-zero
   status, which is why every bash recipe here sets it.

2. `tail -n1` across a multi-part result returns the last part, not the whole.
   `cargo test -p capsem-service` runs three test binaries; the last prints
   `0 passed`, so `| tail -1` reads as though the crate had no tests at all
   while 91 and 264 passed above it.

This guards committed scripts, workflows, and recipes. It cannot guard an
agent's ad-hoc shell -- the rule for that lives in `/dev-testing` -- so it is a
regression guard rather than a detector, and it asserts it actually inspected
something so it cannot pass by finding nothing.
"""

from __future__ import annotations

import re
from collections import defaultdict
from copy import deepcopy
from pathlib import Path

import pytest
import yaml
from helpers.workflow_contract import (
    RequiredJustStep,
    assert_required_just_steps,
    assert_unmasked_step,
)

PROJECT_ROOT = Path(__file__).resolve().parent.parent

# A command whose exit status is the thing being measured, piped into a reader
# that discards it.
GATE_PIPED_TO_READER = re.compile(
    r"(?:cargo\s+(?:test|clippy|build|check)|pytest|uv\s+run\s+pytest|just\s+[a-z_][a-z0-9_-]*)"
    r"[^|\n]*\|\s*(?:head|tail)\b"
)

SHELL_SOURCES = ("*.sh",)
WORKFLOW_DIR = PROJECT_ROOT / ".github" / "workflows"
DOCKER_FAIL_OPEN = re.compile(r"\|\|\s*(?:true\b|:|echo\b)|;\s*true\b|\bset\s+\+e\b")

REQUIRED_JUST_STEPS = (
    RequiredJustStep(
        "ci.yaml",
        "test-linux",
        "Unit tests (KVM backend) with coverage",
        ("just _gate-linux-rust",),
    ),
    RequiredJustStep(
        "fast-gate.yaml",
        "static",
        "Run complete shared fast module",
        ("just _test-fast",),
    ),
    RequiredJustStep(
        "fast-gate.yaml",
        "static",
        "Run shared static module",
        ("just _test-static",),
    ),
    RequiredJustStep(
        "release-assets.yaml",
        "build-assets",
        "Build VM assets (kernel + rootfs)",
        (
            'just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"',
            'just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"',
        ),
    ),
    RequiredJustStep(
        "release-assets.yaml",
        "test-profile-pairing",
        "Run shared artifact module",
        ("just _test-artifacts",),
    ),
    RequiredJustStep(
        "release-assets.yaml",
        "test-profile-pairing",
        "Run shared complete functional module",
        ("just _test-functional",),
        "${{ needs.author-profile-release.outputs.activation_ready == 'true' }}",
    ),
    RequiredJustStep(
        "release-assets.yaml",
        "test-profile-pairing",
        "Run shared native and update glow-up module",
        ("just _test-glowup",),
        "${{ needs.author-profile-release.outputs.activation_ready == 'true' }}",
    ),
    RequiredJustStep(
        "release-nightly.yaml",
        "release-profiles",
        "Rebuild nightly ${{ matrix.profile }} profile assets",
        ("just release-profile nightly ${{ matrix.profile }}",),
    ),
    RequiredJustStep(
        "release-nightly.yaml",
        "release-binaries",
        "Rebuild or release nightly binaries",
        ("just release-binaries nightly",),
    ),
    RequiredJustStep(
        "release.yaml",
        "test-binary-pairing",
        "Run shared artifact module",
        ("just _test-artifacts",),
    ),
    RequiredJustStep(
        "release.yaml",
        "test-binary-pairing",
        "Run shared complete functional module",
        ("just _test-functional",),
    ),
    RequiredJustStep(
        "release.yaml",
        "test-binary-pairing",
        "Run shared native and update glow-up module",
        ("just _test-glowup",),
    ),
)


def _recipe_blocks() -> dict[str, str]:
    """Every justfile recipe body, keyed by recipe name."""
    text = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    blocks = re.split(r"\n(?=[a-z_@][a-zA-Z0-9_-]*(?: [^\n]*)?:)", text)
    named = {}
    for block in blocks:
        head = block.split(":", 1)[0].split()
        if head:
            named[head[0]] = block
    return named


def _docker_instructions(path: Path) -> tuple[str, ...]:
    instructions: list[str] = []
    current = ""
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not current and (not stripped or stripped.startswith("#")):
            continue
        current += (" " if current else "") + stripped.removesuffix("\\").rstrip()
        if not stripped.endswith("\\"):
            instructions.append(" ".join(current.split()))
            current = ""
    assert not current, f"{path}: dangling Dockerfile continuation"
    return tuple(instructions)


def _docker_fail_open_kind(instruction: str) -> str:
    if "cargo llvm-cov show-env" in instruction:
        return "coverage-cache-prewarm"
    if instruction.startswith("RUN cargo build --locked --workspace --all-targets || true"):
        return "ordinary-cache-prewarm"
    if "userdel -r" in instruction:
        return "missing-user-cleanup"
    if "find / -xdev" in instruction and "chmod u-s,g-s" in instruction:
        return "setid-cleanup"
    return f"UNCLASSIFIED: {instruction}"


def test_no_recipe_reads_a_gate_result_through_head_or_tail() -> None:
    recipes = _recipe_blocks()
    assert len(recipes) > 20, "justfile parse found too few recipes to be trusted"

    offenders = {}
    for name, body in recipes.items():
        if match := GATE_PIPED_TO_READER.search(body):
            offenders[name] = match.group(0)

    assert not offenders, (
        "these recipes read a gate's result through head/tail, which reports the "
        "reader's success rather than the gate's:\n  "
        + "\n  ".join(f"{name}: {snippet}" for name, snippet in sorted(offenders.items()))
    )


def test_no_script_or_workflow_reads_a_gate_result_through_head_or_tail() -> None:
    inspected = 0
    offenders = []
    sources = list((PROJECT_ROOT / "scripts").glob("*.sh"))
    sources += sorted(WORKFLOW_DIR.glob("*.yaml"))
    for path in sources:
        inspected += 1
        for match in GATE_PIPED_TO_READER.finditer(path.read_text(encoding="utf-8")):
            offenders.append(f"{path.relative_to(PROJECT_ROOT)}: {match.group(0)}")

    assert inspected > 10, "found too few scripts and workflows to inspect"
    assert not offenders, (
        "a gate's exit status must not be discarded by a pager:\n  " + "\n  ".join(offenders)
    )


def test_every_piping_bash_recipe_sets_pipefail() -> None:
    """Without `pipefail` a failing command upstream of a pipe is invisible."""
    offenders = [
        name
        for name, body in _recipe_blocks().items()
        if "#!/bin/bash" in body and "|" in body and "pipefail" not in body
    ]

    assert not offenders, (
        "these bash recipes pipe without `set -o pipefail`, so a failure "
        "upstream of the pipe is reported as success: " + ", ".join(sorted(offenders))
    )


def test_dockerfile_fail_open_instructions_are_an_exact_reviewed_inventory() -> None:
    sources = sorted((PROJECT_ROOT / "docker").glob("Dockerfile*"))
    sources += sorted((PROJECT_ROOT / "config/docker").rglob("Dockerfile*.j2"))
    found: defaultdict[str, list[str]] = defaultdict(list)
    for path in sources:
        relative = path.relative_to(PROJECT_ROOT).as_posix()
        for instruction in _docker_instructions(path):
            if DOCKER_FAIL_OPEN.search(instruction):
                found[relative].append(_docker_fail_open_kind(instruction))

    assert dict(found) == {
        "docker/Dockerfile.install-builder": ["missing-user-cleanup"],
        "docker/Dockerfile.linux-rust-base": [
            "ordinary-cache-prewarm",
            "coverage-cache-prewarm",
        ],
    }


def _fixture_workflow(step: str, *, job_policy: str = "") -> dict:
    return yaml.safe_load(
        f"jobs:\n  build:\n    runs-on: ubuntu-latest\n{job_policy}    steps:\n{step}"
    )


def _assert_fixture(step: str, *, job_policy: str = "") -> None:
    assert_required_just_steps(
        {"fixture.yaml": _fixture_workflow(step, job_policy=job_policy)},
        (
            RequiredJustStep(
                "fixture.yaml",
                "build",
                "Build assets",
                (
                    'just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"',
                    'just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"',
                ),
            ),
        ),
    )


@pytest.mark.parametrize(
    "step",
    (
        "    - name: Build assets\n"
        "      run: |\n"
        '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
        '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
        "    - name: Build assets\n"
        "      continue-on-error: false\n"
        '      run: "just   _build-kernel   ${{ matrix.arch }}  \\"${{ inputs.profile }}\\"\\n'
        'just _build-rootfs ${{ matrix.arch }} \\"${{ inputs.profile }}\\""\n',
        "    - name: Build assets\n"
        "      if: true\n"
        "      run: |\n"
        "        # YAML and shell presentation are not the contract.\n"
        "        just \\\n"
        '          _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
        '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
    ),
)
def test_required_workflow_commands_accept_equivalent_yaml_and_shell_forms(step: str) -> None:
    _assert_fixture(step)


@pytest.mark.parametrize(
    ("step", "job_policy"),
    (
        (
            "    - name: Build assets\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}" || true\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "",
        ),
        (
            "    - name: Build assets\n"
            "      continue-on-error: true\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "",
        ),
        (
            "    - name: Build assets\n"
            "      if: false\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "",
        ),
        (
            "    - name: Build assets\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "    continue-on-error: true\n",
        ),
        (
            "    - name: Build assets\n"
            "      run: |\n"
            "        set +e\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "",
        ),
        (
            "    - name: Build assets\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}" ; true\n',
            "",
        ),
        (
            "    - name: Build assets\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "",
        ),
        (
            "    - name: Build assets\n"
            "      run: |\n"
            '        just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"\n'
            '        just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n',
            "    if: false\n",
        ),
    ),
)
def test_required_workflow_commands_reject_fail_open_mutations(step: str, job_policy: str) -> None:
    with pytest.raises(AssertionError):
        _assert_fixture(step, job_policy=job_policy)


def test_every_workflow_just_command_is_declared_required_and_unmasked() -> None:
    assert_required_just_steps(_workflow_documents(), REQUIRED_JUST_STEPS)


def _workflow_documents() -> dict[str, dict]:
    return {
        path.name: yaml.safe_load(path.read_text(encoding="utf-8"))
        for path in sorted(WORKFLOW_DIR.glob("*.yaml"))
    }


def _asset_build_step(workflows: dict[str, dict]) -> dict:
    steps = workflows["release-assets.yaml"]["jobs"]["build-assets"]["steps"]
    return next(step for step in steps if step.get("name") == "Build VM assets (kernel + rootfs)")


def test_repository_guard_rejects_the_reviewers_actual_fail_open_mutations() -> None:
    original = _workflow_documents()

    masked_shell = deepcopy(original)
    step = _asset_build_step(masked_shell)
    step["run"] = step["run"].replace(
        'just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}"',
        'just _build-kernel ${{ matrix.arch }} "${{ inputs.profile }}" || true',
    )
    with pytest.raises(AssertionError):
        assert_required_just_steps(masked_shell, REQUIRED_JUST_STEPS)

    ignored_step = deepcopy(original)
    _asset_build_step(ignored_step)["continue-on-error"] = True
    with pytest.raises(AssertionError):
        assert_required_just_steps(ignored_step, REQUIRED_JUST_STEPS)

    removed_proof = deepcopy(original)
    step = _asset_build_step(removed_proof)
    step["run"] = step["run"].replace(
        'just _build-rootfs ${{ matrix.arch }} "${{ inputs.profile }}"\n', ""
    )
    with pytest.raises(AssertionError):
        assert_required_just_steps(removed_proof, REQUIRED_JUST_STEPS)

    unclassified = deepcopy(original)
    unclassified["ci.yaml"]["jobs"]["test"]["steps"].append(
        {"name": "Unclassified shortcut", "run": "just _test-fast"}
    )
    with pytest.raises(AssertionError):
        assert_required_just_steps(unclassified, REQUIRED_JUST_STEPS)


def test_required_workflow_pipeline_must_enable_pipefail_first() -> None:
    unprotected = _fixture_workflow(
        "    - name: Build assets\n      run: cargo test --locked 2>&1 | tee test.log\n"
    )
    with pytest.raises(AssertionError, match="pipeline can discard enforcement status"):
        assert_unmasked_step("fixture.yaml", unprotected, "build", "Build assets")

    protected = _fixture_workflow(
        "    - name: Build assets\n"
        "      run: |\n"
        "        set -o pipefail\n"
        "        cargo test --locked 2>&1 | tee test.log\n"
    )
    assert_unmasked_step("fixture.yaml", protected, "build", "Build assets")
