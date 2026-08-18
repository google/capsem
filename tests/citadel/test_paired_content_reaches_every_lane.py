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

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

WORKFLOWS = ROOT / ".github/workflows"

#: The recipes whose plans contain `glowup.content`.
PROVES_PAIRED_CONTENT = ("just qualify-binaries", "just qualify-assets")

MATERIALIZE = "bash scripts/materialize-config.sh"


def _jobs(workflow: Path) -> dict[str, str]:
    """Each top-level job of a workflow, as text.

    Sliced rather than parsed: the subject is what a job's `run:` blocks say,
    and a YAML load would give back the same strings after discarding the line
    numbers that make a failure findable.
    """
    text = workflow.read_text(encoding="utf-8")
    starts = [match.start() for match in re.finditer(r"^  (?P<name>[a-z][a-z0-9-]*):$", text, re.M)]
    names = re.findall(r"^  ([a-z][a-z0-9-]*):$", text, re.M)
    bounds = [*starts, len(text)]
    return {name: text[bounds[index] : bounds[index + 1]] for index, name in enumerate(names)}


def _qualifying_jobs() -> dict[str, str]:
    found = {
        f"{workflow.name}:{name}": body
        for workflow in sorted(WORKFLOWS.glob("*.yaml"))
        for name, body in _jobs(workflow).items()
        if any(recipe in body for recipe in PROVES_PAIRED_CONTENT)
    }
    assert found, "no workflow job qualifies a release, so this guard watches nothing"
    return found


def test_every_qualifying_job_pairs_the_content_it_materializes() -> None:
    """The flag itself, in the jobs whose plans compare the two manifests."""
    unpaired = [
        f"{label}: {line.strip()}"
        for label, body in _qualifying_jobs().items()
        for line in body.splitlines()
        if MATERIALIZE in line and "--pair-content" not in line
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
        f"{label}: {line.strip()}"
        for label, body in _qualifying_jobs().items()
        if MATERIALIZE in body
        for line in body.splitlines()
        if "CAPSEM_ASSET_MANIFEST=" in line and "file://" in line
    ]
    assert not urls, (
        "a paired materialization compares its selected manifest against the "
        "assets directory as filesystem paths:\n  " + "\n  ".join(urls)
    )
