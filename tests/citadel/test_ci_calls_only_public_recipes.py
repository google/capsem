"""No workflow may invoke a private `just` recipe.

The justfile header says underscore recipes are implementation detail, and it
used to carve out an exception: "CI may call a private primitive only when it
is part of the canonical `test` graph". Every release workflow then took it.

The cost was not the broken convention, it was where the integration ended up.
With no public verb meaning "qualify this lane", each workflow assembled the
lane itself: three or four private steps, in an order restated in YAML, with
the deferred-profile branch written as a step-level `if:`. The module bodies
were shared; the sequence around them was not. That is how the asset lane grew
a `_test-profile-artifacts` branch the binary lane never got -- a divergence no
test could see, because each half was individually correct.

So the exception is gone. A workflow calls a public recipe or it calls none.
If CI needs something the public surface does not offer, the answer is a new
public recipe, reviewed through `config/public-surface.toml`, not a reach into
the internals.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

import pytest
from helpers.workflow_contract import just_recipe_names, workflow_jobs

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((PROJECT_ROOT / ".github/workflows").glob("*.yaml"))

def _approved() -> set[str]:
    document = tomllib.loads(
        (PROJECT_ROOT / "config/public-surface.toml").read_text(encoding="utf-8")
    )
    return set(document["just"]["approved"])


def _calls(workflow: Path) -> set[str]:
    return {
        recipe
        for job_name, job in workflow_jobs(workflow).items()
        for index, step in enumerate(job.get("steps") or ())
        if isinstance(step, dict) and isinstance(step.get("run"), str)
        for recipe in just_recipe_names(
            step["run"], origin=f"{workflow.name}:{job_name}:{index}"
        )
    }


def test_the_workflow_inventory_is_not_empty() -> None:
    """Without this the guard passes by finding nothing to check."""
    assert len(WORKFLOWS) >= 5
    assert any(_calls(path) for path in WORKFLOWS)


@pytest.mark.parametrize("workflow", WORKFLOWS, ids=lambda path: path.name)
def test_no_workflow_invokes_a_private_recipe(workflow: Path) -> None:
    private = sorted(name for name in _calls(workflow) if name.startswith("_"))

    assert not private, (
        f"{workflow.name} calls private recipes {private}. Underscore recipes are "
        "implementation detail; a workflow needing one needs a public verb "
        "instead, approved in config/public-surface.toml."
    )


@pytest.mark.parametrize("workflow", WORKFLOWS, ids=lambda path: path.name)
def test_every_recipe_a_workflow_calls_is_on_the_public_surface(workflow: Path) -> None:
    """Public and private are not the only two states: a recipe can simply not
    exist, and a workflow naming one fails only when that job finally runs."""
    approved = _approved()
    called = _calls(workflow)

    unknown = sorted(name for name in called if name not in approved)
    assert not unknown, (
        f"{workflow.name} calls {unknown}, which the locked public surface does "
        "not list. Add them to config/public-surface.toml with its count, or "
        "call an approved recipe."
    )
