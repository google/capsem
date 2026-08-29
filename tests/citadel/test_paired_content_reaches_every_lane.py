"""Citadel guard: a lane that proves paired content has to be handed a pair.

`glowup.content` compares the staged asset manifest against the materialized
runtime one byte for byte. They are not the same document until
`materialize-config.sh --pair-content` makes them so: staging writes the channel
*graph* to `assets/manifest.json`, and materializing writes the legacy runtime
projection under `config/assets/`. Without the flag the two differ by three
orders of magnitude in size and the step fails.

The binary pairing job omitted it. The profile lane and the local install lane
both pass it, so the flag was clearly known and clearly required -- it was
simply absent from the one job no local run reproduces, which is the same shape
as every other defect this directory records about that job. It was found by
running the release lane's own staging locally rather than by reading the YAML.

Two properties, because the flag alone is not enough: `--pair-content` compares
its two arguments as filesystem paths, so the selected manifest has to be a
path and the assets directory has to be named beside it. A `file://` URL there
fails the comparison against the very manifest it was given.
"""

from __future__ import annotations

from pathlib import Path

from helpers.workflow_contract import (
    just_recipe_names,
    parsed_commands,
    workflow_jobs,
    workflow_reachable_shell,
)

ROOT = Path(__file__).resolve().parents[2]

WORKFLOWS = ROOT / ".github/workflows"

#: The recipes whose plans contain `glowup.content`.
PROVES_PAIRED_CONTENT = {"qualify-binaries", "qualify-assets"}

MATERIALIZE = "build_system/scripts/build/materialize-config.sh"


def _qualifying_jobs() -> dict[str, str]:
    found = {}
    for workflow in sorted(WORKFLOWS.glob("*.yaml")):
        for name, job in workflow_jobs(workflow).items():
            runs = [
                step["run"]
                for step in job.get("steps") or ()
                if isinstance(step, dict) and isinstance(step.get("run"), str)
            ]
            if not any(
                PROVES_PAIRED_CONTENT.intersection(
                    just_recipe_names(run, origin=f"{workflow.name}:{name}")
                )
                for run in runs
            ):
                continue
            found[f"{workflow.name}:{name}"] = workflow_reachable_shell(
                ROOT, workflow, job=name
            )
    assert found, "no workflow job qualifies a release, so this guard watches nothing"
    return found


def _materializers(label: str, shell: str):
    return [
        command
        for command in parsed_commands(shell, origin=label)
        if MATERIALIZE in command.argv
    ]


def test_every_qualifying_job_pairs_the_content_it_materializes() -> None:
    """The flag itself, in the jobs whose plans compare the two manifests."""
    unpaired = [
        f"{label}: {' '.join(command.argv)}"
        for label, body in _qualifying_jobs().items()
        for command in _materializers(label, body)
        if "--pair-content" not in command.argv
    ]
    assert not unpaired, (
        "these jobs qualify a release and materialize configuration without "
        "pairing it, so `glowup.content` compares a channel graph against a "
        "runtime projection and fails:\n  " + "\n  ".join(unpaired)
    )


def test_the_paired_manifest_is_named_as_a_path_and_not_a_url() -> None:
    """`--pair-content` compares paths, so a URL fails against itself.

    The message it produces names the same file twice in two spellings, which
    reads like a mismatch in the content rather than in how it was addressed.
    """
    urls = [
        f"{label}: {assignment}"
        for label, body in _qualifying_jobs().items()
        for command in _materializers(label, body)
        for assignment in command.assignments
        if assignment.startswith("CAPSEM_ASSET_MANIFEST=") and "file://" in assignment
    ]
    assert not urls, (
        "a paired materialization compares its selected manifest against the "
        "assets directory as filesystem paths:\n  " + "\n  ".join(urls)
    )
