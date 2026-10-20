"""Image publishers must notice their defining inputs and rebuild regularly."""

from __future__ import annotations

import fnmatch
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
TRIGGER_RATIONALE = (
    "Image catalogs and builder generations become stale when publication only "
    "watches Dockerfiles. Keep scheduled rebuilds, manual stable GA selection, "
    "and main-push coverage for the shared evidence and helper inputs."
)
INPUTS = {
    "images.yaml": (
        ".github/workflows/images.yaml",
        "images/dev/Dockerfile",
        "build_system/builder/image/docker.py",
        "build_system/builder/release/obom.py",
        "build_system/docker/Dockerfile.asset-tools",
        "config/docker/image/build.toml",
        "config/gate.toml",
        "build_system/pyproject.toml",
        "build_system/uv.lock",
    ),
    "private-images.yaml": (
        ".github/workflows/private-images.yaml",
        "build_system/docker/materialize-host-apt.sh",
        "build_system/builder/privateimages.py",
        "config/docker/image/build.toml",
        "Cargo.lock",
        "web/app/pnpm-lock.yaml",
    ),
}


def check_triggers(document: dict, inputs: tuple[str, ...]) -> None:
    events = document.get("on", document.get(True, {}))
    assert events.get("schedule") and all(row.get("cron") for row in events["schedule"]), (
        TRIGGER_RATIONALE
    )
    assert "workflow_dispatch" in events, TRIGGER_RATIONALE
    push = events.get("push", {})
    assert "main" in push.get("branches", []), TRIGGER_RATIONALE
    for path in inputs:
        assert any(fnmatch.fnmatchcase(path, pattern) for pattern in push.get("paths", [])), (
            f"{path}: {TRIGGER_RATIONALE}"
        )


@pytest.mark.parametrize("name", INPUTS)
def test_publishers_rebuild_for_shared_inputs_and_on_schedule(name: str) -> None:
    check_triggers(yaml.safe_load((ROOT / ".github/workflows" / name).read_text()), INPUTS[name])


@pytest.mark.parametrize("removed", ["schedule", "workflow_dispatch", "push"])
def test_missing_automatic_or_ga_entrypoint_is_rejected(removed: str) -> None:
    events = {
        "schedule": [{"cron": "17 3 * * 3"}],
        "workflow_dispatch": {},
        "push": {"branches": ["main"], "paths": ["**"]},
    }
    events.pop(removed)
    with pytest.raises(AssertionError, match="stale"):
        check_triggers({"on": events}, ("build_system/builder/image/docker.py",))
