"""Citadel guard: every downloaded artifact is uploaded by a job that ran first.

Artifacts are how one job hands work to the next, and a release is mostly that:
seven handoffs across the five publication jobs alone. Nothing checked them.
A download naming an artifact nobody uploads, or uploaded by a job this one
does not wait for, fails at the moment of download -- which for the publication
jobs is after the binaries are built, the packages installed and the pairing
gate passed, an hour into a release nobody can rerun cheaply.

None of those five jobs has ever executed. Their scripts are covered by tests;
the wiring between them is not, and wiring is what has failed all day: a tool
absent from one job's list, a directory nothing staged, a variable a child no
longer saw.

Checked here because it is decidable from the file: the upload exists or it
does not, and the job that performs it either precedes this one or does not.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((PROJECT_ROOT / ".github" / "workflows").glob("*.yaml"))

UPLOAD = "actions/upload-artifact"
DOWNLOAD = "actions/download-artifact"


def _steps(job: dict) -> list[dict]:
    return [step for step in (job.get("steps") or []) if isinstance(step, dict)]


def _matrix_values(job: dict) -> dict[str, list[str]]:
    """The literal matrix axes a job fans out over."""
    matrix = ((job.get("strategy") or {}).get("matrix")) or {}
    axes: dict[str, list[str]] = {}
    if isinstance(matrix, dict):
        for key, values in matrix.items():
            if key in {"include", "exclude"} or not isinstance(values, list):
                continue
            literal = [str(value) for value in values if isinstance(value, str | int)]
            if literal:
                axes[key] = literal
        # `include` is how this repository spells its matrices -- `build-app-linux`
        # pairs each arch with its runner that way -- so an axis read only from
        # the top level finds nothing and the guard reports every expanded
        # download as unresolved.
        for entry in matrix.get("include") or []:
            if not isinstance(entry, dict):
                continue
            for key, value in entry.items():
                if isinstance(value, str | int):
                    axes.setdefault(key, [])
                    if str(value) not in axes[key]:
                        axes[key].append(str(value))
    return axes


def _expand(name: str, axes: dict[str, list[str]]) -> set[str]:
    """Every concrete name a matrix-templated artifact resolves to.

    `build-app-linux` uploads `release-linux-${{ matrix.arch }}`, and three jobs
    download the expanded names. Skipping templated names as unknowable made
    the guard report those three as unresolved, which is a guard that has to be
    switched off rather than believed.
    """
    names = {name}
    for key, values in axes.items():
        token = "${{ matrix." + key + " }}"
        if not any(token in candidate for candidate in names):
            continue
        names = {candidate.replace(token, value) for candidate in names for value in values}
    return {candidate for candidate in names if "${{" not in candidate}


def _artifacts(job: dict, action: str) -> set[str]:
    """Artifact names a job uploads or downloads, matrix axes expanded.

    A name built from a job *output* still cannot be paired here; those resolve
    only at run time and are left to the run rather than guessed at.
    """
    axes = _matrix_values(job)
    found: set[str] = set()
    for step in _steps(job):
        uses = step.get("uses") or ""
        if not uses.startswith(action):
            continue
        name = (step.get("with") or {}).get("name")
        if isinstance(name, str):
            found |= _expand(name, axes)
    return found


def _ancestors(jobs: dict[str, dict], name: str) -> set[str]:
    """Every job that must finish before `name`, transitively."""
    seen: set[str] = set()
    frontier = list(_needs(jobs.get(name) or {}))
    while frontier:
        current = frontier.pop()
        if current in seen or current not in jobs:
            continue
        seen.add(current)
        frontier.extend(_needs(jobs[current]))
    return seen


def _needs(job: dict) -> list[str]:
    needs = job.get("needs")
    if isinstance(needs, str):
        return [needs]
    return [n for n in (needs or []) if isinstance(n, str)]


def _workflow_jobs(path: Path) -> dict[str, dict]:
    loaded = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    jobs = loaded.get("jobs")
    return {k: v for k, v in (jobs or {}).items() if isinstance(v, dict)}


def test_the_guard_sees_real_handoffs() -> None:
    """A rule over no artifacts asserts nothing."""
    total = 0
    for path in WORKFLOWS:
        jobs = _workflow_jobs(path)
        total += sum(len(_artifacts(job, DOWNLOAD)) for job in jobs.values())
    assert total >= 5, f"only {total} static artifact downloads found; the shape has drifted"


@pytest.mark.parametrize("workflow", WORKFLOWS, ids=lambda path: path.name)
def test_every_downloaded_artifact_is_uploaded_by_an_earlier_job(workflow: Path) -> None:
    """The download either resolves or the release dies an hour in."""
    jobs = _workflow_jobs(workflow)
    unresolved = []
    for name, job in jobs.items():
        wanted = _artifacts(job, DOWNLOAD)
        if not wanted:
            continue
        earlier = _ancestors(jobs, name)
        # A job may also consume what it produced itself.
        available = _artifacts(job, UPLOAD)
        for ancestor in earlier:
            available |= _artifacts(jobs[ancestor], UPLOAD)
        for missing in sorted(wanted - available):
            producers = sorted(
                other for other, candidate in jobs.items() if missing in _artifacts(candidate, UPLOAD)
            )
            unresolved.append(
                f"{name} downloads {missing!r}, "
                + (
                    f"which only {producers} upload and it does not wait for"
                    if producers
                    else "which no job in this workflow uploads"
                )
            )
    assert not unresolved, (
        f"{workflow.name}: " + "; ".join(unresolved)
    )
