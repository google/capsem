"""Citadel guard: only the image workflow publishes images, and it does so alone.

The official OCI images under `images/` are built and pushed by
`.github/workflows/images.yaml`, and by nothing else. Why, and what each
check below refuses, is in `IMAGE_PUBLICATION_RATIONALE` -- stated there so a
violation prints it rather than a bare assertion.

Every predicate here is pure over workflow text, so the adversarial cases at
the bottom drive the same code the sweep does.
"""

from __future__ import annotations

import re
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import pytest
import yaml
from helpers.workflow_contract import canonical_shell_commands

IMAGE_PUBLICATION_RATIONALE = """\
Official images are published by .github/workflows/images.yaml and nothing else.

`packages: write` lets a job push, overwrite or retag anything under
ghcr.io/google/capsem -- the images every Capsem session boots. A token that
can do that belongs to one workflow, granted per publishing job, so the set of
code that can replace a published image is one file a reviewer can read.
Granted to a trunk workflow, every PR-triggered dependency, action and script
in that workflow inherits it. `write-all` grants it too, and counts.

Trunk workflows (ci.yaml, fast-gate.yaml, release*.yaml, ...) do not build or
push images from images/ either. An image built beside trunk work is an image
published without the input key, smoke test, OBOM, EROFS, provenance and
append-only catalog that images.yaml attaches to every version, and a second
publisher is a second opinion about what is current.

images.yaml never shares a concurrency group with another workflow. A shared
group would queue a PR's gate behind a multi-hour image build, or -- with
cancel-in-progress -- let one cancel the other half way through a catalog
publication. Its group is a literal, so no expression can evaluate into a
trunk group, and no trunk group's literal prefix may cover it.

See .github/workflows/images.yaml and images/ci/.
"""

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"
IMAGES_WORKFLOW = "images.yaml"
REGISTRY = "ghcr.io/google/capsem"
EXPRESSION = "${{"

#: `images/` as a path from the repository root, however it is reached: bare,
#: `./images/`, `$GITHUB_WORKSPACE/images/`, `${{ github.workspace }}/images/`.
#: Not `web/graphics/images/`, which is somebody else's directory.
IMAGES_PATH = re.compile(r"(?:^|[\s\"'=:(,\[]|}/|\$\{?\w+\}?/|\./)images/")
#: A shell word naming the tree itself: `cd images`, `-C ./images`.
IMAGES_WORDS = frozenset({"images", "./images", "images/"})


def workflow_paths() -> list[Path]:
    return sorted([*WORKFLOWS.glob("*.yaml"), *WORKFLOWS.glob("*.yml")])


def _document(text: str) -> dict[str, Any]:
    document = yaml.safe_load(text) or {}
    assert isinstance(document, dict), "a workflow must be a mapping"
    return document


def _jobs(document: dict[str, Any]) -> dict[str, Any]:
    jobs = document.get("jobs") or {}
    return {name: job for name, job in jobs.items() if isinstance(job, dict)}


def _grants_packages_write(permissions: object) -> bool:
    if isinstance(permissions, str):
        return permissions.strip() == "write-all"
    return isinstance(permissions, dict) and permissions.get("packages") == "write"


def packages_write_grants(text: str) -> list[str]:
    """Where a workflow grants `packages: write`: `workflow` or `job <name>`."""
    document = _document(text)
    found = ["workflow"] if _grants_packages_write(document.get("permissions")) else []
    found += [
        f"job {name}"
        for name, job in _jobs(document).items()
        if _grants_packages_write(job.get("permissions"))
    ]
    return found


def _strings(value: object) -> Iterator[str]:
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, item in value.items():
            yield from _strings(key)
            yield from _strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from _strings(item)


def _run_bodies(document: dict[str, Any]) -> Iterator[str]:
    for job in _jobs(document).values():
        for step in job.get("steps") or ():
            if isinstance(step, dict) and isinstance(step.get("run"), str):
                yield step["run"]


def _names_images_tree(word: str) -> bool:
    return any(part in IMAGES_WORDS for part in word.split("="))


def official_image_references(text: str) -> list[str]:
    """Every place a workflow reaches into images/, its registry, or images.yaml."""
    document = _document(text)
    found = sorted(
        {
            value.strip().splitlines()[0] if value.strip() else value
            for value in _strings(document)
            if IMAGES_PATH.search(value)
            or REGISTRY in value
            or f"workflows/{IMAGES_WORKFLOW}" in value
        }
    )
    for body in _run_bodies(document):
        for command in canonical_shell_commands(body):
            if any(_names_images_tree(word) for word in command):
                found.append(" ".join(command))
    return found


def concurrency_groups(text: str) -> list[str]:
    """Every concurrency group a workflow declares, workflow- or job-level."""
    document = _document(text)
    holders = [document, *_jobs(document).values()]
    groups = []
    for holder in holders:
        concurrency = holder.get("concurrency")
        if isinstance(concurrency, dict):
            concurrency = concurrency.get("group")
        if isinstance(concurrency, str):
            groups.append(concurrency.strip())
    return groups


def colliding_groups(images: list[str], other: list[str]) -> list[str]:
    """Groups of `other` that equal, or could evaluate to, a group in `images`."""
    collisions = []
    for group in other:
        literal = group.split(EXPRESSION, 1)[0]
        for mine in images:
            if mine == group or (EXPRESSION in group and mine.startswith(literal)):
                collisions.append(f"{group} vs {mine}")
    return collisions


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _trunk() -> list[Path]:
    return [path for path in workflow_paths() if path.name != IMAGES_WORKFLOW]


def test_the_sweep_sees_the_workflows() -> None:
    """A guard over an empty directory, or one missing images.yaml, holds nothing."""
    names = {path.name for path in workflow_paths()}
    assert IMAGES_WORKFLOW in names
    assert {"ci.yaml", "fast-gate.yaml", "release.yaml"} <= names


@pytest.mark.parametrize("workflow", _trunk(), ids=lambda path: path.name)
def test_no_trunk_workflow_can_write_packages(workflow: Path) -> None:
    grants = packages_write_grants(_read(workflow))
    assert not grants, (
        IMAGE_PUBLICATION_RATIONALE + f"\n{workflow.name} grants packages: write at {grants}."
    )


def test_the_image_workflow_writes_packages_only_where_it_publishes() -> None:
    text = _read(WORKFLOWS / IMAGES_WORKFLOW)
    grants = packages_write_grants(text)
    assert "workflow" not in grants, (
        IMAGE_PUBLICATION_RATIONALE
        + "\nimages.yaml must default to read-only and grant packages: write per job."
    )
    assert grants, "images.yaml publishes images, so some job must hold packages: write"


@pytest.mark.parametrize("workflow", _trunk(), ids=lambda path: path.name)
def test_no_trunk_workflow_builds_official_images(workflow: Path) -> None:
    references = official_image_references(_read(workflow))
    assert not references, (
        IMAGE_PUBLICATION_RATIONALE + f"\n{workflow.name} reaches the official images: {references}"
    )


def test_the_image_workflow_shares_no_concurrency_group() -> None:
    images = concurrency_groups(_read(WORKFLOWS / IMAGES_WORKFLOW))
    assert images, IMAGE_PUBLICATION_RATIONALE + "\nimages.yaml must declare its own group."
    assert all(EXPRESSION not in group for group in images), (
        IMAGE_PUBLICATION_RATIONALE + f"\nimages.yaml groups must be literal: {images}"
    )
    collisions = [
        f"{path.name}: {collision}"
        for path in _trunk()
        for collision in colliding_groups(images, concurrency_groups(_read(path)))
    ]
    assert not collisions, IMAGE_PUBLICATION_RATIONALE + f"\nShared groups: {collisions}"


# --- adversarial cases: the predicates above must see each of these ---------

JOB_LEVEL_WRITE = """
on: push
permissions:
  contents: read
jobs:
  build:
    runs-on: ubuntu-24.04
    permissions:
      contents: read
      packages: write
    steps:
      - run: echo hi
"""


@pytest.mark.parametrize(
    ("text", "where"),
    [
        (JOB_LEVEL_WRITE, ["job build"]),
        ("on: push\npermissions: write-all\njobs: {}\n", ["workflow"]),
        ("on: push\npermissions:\n  packages: write\njobs: {}\n", ["workflow"]),
        (JOB_LEVEL_WRITE.replace("packages: write", "packages: read"), []),
    ],
)
def test_packages_write_is_seen_wherever_it_is_granted(text: str, where: list[str]) -> None:
    assert packages_write_grants(text) == where


def _with_run(body: str) -> str:
    return yaml.safe_dump(
        {"on": "push", "jobs": {"build": {"runs-on": "x", "steps": [{"run": body}]}}}
    )


@pytest.mark.parametrize(
    "body",
    [
        "docker buildx build images/dev",
        "docker buildx build --push -t x ./images/dev",
        "docker build -f images/dev/Dockerfile .",
        "docker build --file=images/dev/Dockerfile .",
        'docker build "$GITHUB_WORKSPACE/images/agy"',
        "cd images && docker build dev",
        "make -C ./images",
        "docker push ghcr.io/google/capsem/dev:latest",
        "sh images/smoke.sh x",
    ],
)
def test_building_official_images_is_seen_in_any_spelling(body: str) -> None:
    assert official_image_references(_with_run(body)), body


def test_official_images_are_seen_outside_run_bodies() -> None:
    text = yaml.safe_dump(
        {
            "on": "push",
            "jobs": {
                "build": {
                    "runs-on": "x",
                    "steps": [
                        {"uses": "example/build-push-action@x", "with": {"context": "images/dev"}}
                    ],
                },
                "call": {"uses": "./.github/workflows/images.yaml"},
            },
        }
    )
    assert len(official_image_references(text)) == 2


@pytest.mark.parametrize(
    "body",
    [
        "cp web/graphics/images/logo.png out/",
        "echo 'Build VM images'",
        "ls cache/target/assets/images/",
    ],
)
def test_other_directories_named_images_are_not_official_images(body: str) -> None:
    assert official_image_references(_with_run(body)) == []


@pytest.mark.parametrize(
    ("other", "collides"),
    [
        ("capsem-official-images", True),
        ("capsem-${{ inputs.channel }}", True),
        ("${{ github.workflow }}", True),
        ("capsem-release-${{ inputs.channel }}", False),
        ("ci-${{ github.ref }}", False),
    ],
)
def test_a_shared_or_covering_concurrency_group_is_seen(other: str, collides: bool) -> None:
    text = f"on: push\nconcurrency:\n  group: {other}\njobs: {{}}\n"
    assert bool(colliding_groups(["capsem-official-images"], concurrency_groups(text))) is collides
