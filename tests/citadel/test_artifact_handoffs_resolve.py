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

Checked here because it is decidable from the workflow graph, including local
reusable calls: an upload exists and its owning job precedes the download.
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


def _has_cross_run_resolver(job: dict) -> bool:
    """The job proves which immutable prior run may supply artifacts."""
    commands = [step.get("run") or "" for step in _steps(job)]
    return any(
        isinstance(command, str) and "verify-release-recovery-run.py" in command
        for command in commands
    )


def _needs_output_producer(run_id: str) -> str | None:
    """Return the job named by a ``needs.<job>.outputs.*`` expression."""
    marker = "needs."
    output_marker = ".outputs."
    if marker not in run_id:
        return None
    remainder = run_id.split(marker, 1)[1]
    if output_marker not in remainder:
        return None
    producer = remainder.split(output_marker, 1)[0]
    return producer or None


def _cross_run_downloads(job: dict, jobs: dict[str, dict]) -> set[str]:
    """Authenticated artifacts admitted by an earlier run-identity proof."""
    verified = False
    found: set[str] = set()
    axes = _matrix_values(job)
    for step in _steps(job):
        command = step.get("run") or ""
        if isinstance(command, str) and "verify-release-recovery-run.py" in command:
            verified = True
        uses = step.get("uses") or ""
        if not uses.startswith(DOWNLOAD):
            continue
        inputs = step.get("with") or {}
        run_id = inputs.get("run-id")
        if run_id is None:
            continue
        name = inputs.get("name")
        token = inputs.get("github-token")
        assert isinstance(name, str) and name, "cross-run artifact download needs a literal name"
        assert isinstance(run_id, str) and run_id, f"cross-run download {name!r} needs run-id"
        assert isinstance(token, str) and token, f"cross-run download {name!r} needs github-token"
        producer = _needs_output_producer(run_id)
        upstream_verified = (
            producer in _needs(job) and producer in jobs and _has_cross_run_resolver(jobs[producer])
        )
        assert verified or upstream_verified, (
            f"cross-run download {name!r} has no prior immutable run resolver"
        )
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


def _called_workflow(workflow: Path, job: dict) -> Path | None:
    uses = job.get("uses", "")
    if uses.startswith("./.github/workflows/"):
        return workflow.parent / Path(uses).name
    return None


def _produced(workflow: Path, job: dict, chain: tuple[Path, ...] = ()) -> set[str]:
    child = _called_workflow(workflow, job)
    if child is None:
        return _artifacts(job, UPLOAD)
    assert child not in chain, f"recursive artifact workflow call: {child.name}"
    return set().union(*(_produced(child, row, (*chain, child)) for row in _workflow_jobs(child).values()))


def test_the_guard_sees_real_handoffs() -> None:
    """A rule over no artifacts asserts nothing."""
    total = 0
    for path in WORKFLOWS:
        jobs = _workflow_jobs(path)
        total += sum(len(_artifacts(job, DOWNLOAD)) for job in jobs.values())
    assert total >= 5, f"only {total} static artifact downloads found; the shape has drifted"


def test_cross_run_download_requires_an_immutable_run_resolver() -> None:
    job = {
        "steps": [
            {
                "uses": "actions/download-artifact@pinned",
                "with": {
                    "name": "qualified-candidate",
                    "run-id": "${{ inputs.failed_run_id }}",
                    "github-token": "${{ github.token }}",
                },
            }
        ]
    }
    with pytest.raises(AssertionError, match="no prior immutable run resolver"):
        _cross_run_downloads(job, {"consume": job})


def test_cross_run_download_accepts_a_verified_upstream_matrix_reuse() -> None:
    producer = {"steps": [{"run": "python3 build_system/scripts/release/verify-release-recovery-run.py"}]}
    consumer = {
        "needs": ["resolve"],
        "strategy": {"matrix": {"arch": ["arm64", "x86_64"]}},
        "steps": [
            {
                "uses": "actions/download-artifact@pinned",
                "with": {
                    "name": "vm-assets-${{ matrix.arch }}",
                    "run-id": "${{ needs.resolve.outputs.reuse_run_id }}",
                    "github-token": "${{ github.token }}",
                },
            }
        ],
    }
    assert _cross_run_downloads(consumer, {"resolve": producer}) == {
        "vm-assets-arm64",
        "vm-assets-x86_64",
    }


def _unresolved(workflow: Path, inherited: frozenset[str] = frozenset(), chain: tuple[Path, ...] = ()) -> list[str]:
    assert workflow not in chain, f"recursive artifact workflow call: {workflow.name}"
    jobs = _workflow_jobs(workflow)
    unresolved = []
    for name, job in jobs.items():
        # A local reusable workflow shares this run's artifact namespace, but
        # receives only outputs from the caller's completed dependencies.
        earlier = set(inherited)
        for ancestor in _ancestors(jobs, name):
            earlier |= _produced(workflow, jobs[ancestor], (*chain, workflow))
        child = _called_workflow(workflow, job)
        if child is not None:
            unresolved.extend(_unresolved(child, frozenset(earlier), (*chain, workflow)))
        wanted = _artifacts(job, DOWNLOAD) - _cross_run_downloads(job, jobs)
        if not wanted:
            continue
        # A job may also consume what it produced itself.
        available = earlier | _artifacts(job, UPLOAD)
        for missing in sorted(wanted - available):
            producers = sorted(
                other
                for other, candidate in jobs.items()
                if missing in _produced(workflow, candidate, (*chain, workflow))
            )
            unresolved.append(
                f"{name} downloads {missing!r}, "
                + (
                    f"which only {producers} upload and it does not wait for"
                    if producers
                    else "which no job in this workflow uploads"
                )
            )
    return unresolved


@pytest.mark.parametrize("workflow", WORKFLOWS, ids=lambda path: path.name)
def test_every_downloaded_artifact_is_uploaded_by_an_earlier_job(workflow: Path) -> None:
    """Every entrypoint proves the complete caller/callee artifact graph."""
    document = yaml.safe_load(workflow.read_text())
    triggers = document.get("on", document.get(True, {}))
    callers = [parent for parent in WORKFLOWS for job in _workflow_jobs(parent).values()
               if _called_workflow(parent, job) == workflow]
    if set(triggers) == {"workflow_call"} and callers:
        # The caller's parametrized case recurses into this workflow with its
        # actual dependency outputs, rather than inventing external uploads.
        for caller in callers:
            unresolved = _unresolved(caller)
            assert not unresolved, f"{caller.name}: " + "; ".join(unresolved)
        return
    unresolved = _unresolved(workflow)
    assert not unresolved, f"{workflow.name}: " + "; ".join(unresolved)


@pytest.mark.parametrize("mutation", [None, "missing-edge", "wrong-name", "child-order"])
def test_reusable_artifacts_keep_real_caller_and_child_dependencies(tmp_path: Path, mutation: str | None) -> None:
    parent = tmp_path / "parent.yaml"
    child = tmp_path / "child.yaml"
    upload = {"steps": [{"uses": UPLOAD, "with": {"name": "archive"}}]}
    consume = {"steps": [{"uses": DOWNLOAD, "with": {"name": "archive"}}]}
    child_upload = {"steps": [{"uses": UPLOAD, "with": {"name": "verified"}}]}
    child_consume = {"needs": "prepare", "steps": [{"uses": DOWNLOAD, "with": {"name": "verified"}}]}
    call = {"needs": "build", "uses": "./.github/workflows/child.yaml"}
    if mutation == "missing-edge":
        call.pop("needs")
    elif mutation == "wrong-name":
        upload["steps"][0]["with"]["name"] = "different"
    elif mutation == "child-order":
        child_consume.pop("needs")
    parent.write_text(yaml.safe_dump({"jobs": {"build": upload, "publish": call}}))
    child.write_text(yaml.safe_dump({"jobs": {"consume": consume, "prepare": child_upload, "finish": child_consume}}))
    assert bool(_unresolved(parent)) == bool(mutation)
