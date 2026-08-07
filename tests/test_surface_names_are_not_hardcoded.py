"""Public command names come from `variables`, never from a string literal.

Renaming one public recipe broke five contracts across four files. Not one of
them was testing behaviour: they asserted that a block called `smoke:` held
certain lines in a certain order. So a rename that changed nothing failed the
build, and a behaviour change that kept the name would have passed -- wrong in
both directions, and the reason a two-line rename cost an afternoon.

The rule this enforces is the one the gate already applies to itself in
`test_gate_has_no_literal_data.py`: a name with an owner is read from its
owner. `config/public-surface.toml` owns the public recipe names, `variables`
reads it, and tests ask `variables`. Renaming then touches the ledger, the
justfile and the docs -- and no test at all.

**Scoped to the public surface on purpose.** Forbidding every literal in every
test would be a large refactor bought with no safety, and narrowing afterwards
to whatever made this pass would be worse: a guard shaped around its own
result. The public recipes are the names that (a) change as a product
decision, (b) appear in many files, and (c) have a checked ledger to be read
from. That is the whole class, and it is closed.
"""

from __future__ import annotations

import re
from pathlib import Path

import variables

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: This module and `variables` itself must name them; that is their job.
_ALLOWED = {"variables.py", Path(__file__).name}

#: Only the names ambiguous enough to be worth policing. `test` and `build`
#: are ordinary English and appear constantly in unrelated prose, so matching
#: them would produce noise rather than a guard -- and a rename of those is
#: not the change this exists to survive.
_POLICED = (variables.FAST_TEST, variables.VM_SMOKE)


def _test_sources() -> list[Path]:
    return sorted(
        path
        for path in (PROJECT_ROOT / "tests").rglob("*.py")
        if path.name not in _ALLOWED
    )


def test_no_test_hardcodes_a_public_recipe_name() -> None:
    """A literal here is a test that fails on a rename and passes on a bug."""
    offenders: list[str] = []
    for path in _test_sources():
        text = path.read_text(encoding="utf-8")
        for name in _POLICED:
            for match in re.finditer(rf"[\"']{re.escape(name)}[:\"']", text):
                line = text.count("\n", 0, match.start()) + 1
                offenders.append(f"{path.relative_to(PROJECT_ROOT)}:{line} {name!r}")

    assert not offenders, (
        "these spell a public recipe name as a literal, so renaming it breaks "
        f"them for no behavioural reason: {offenders}. Use "
        "`variables.FAST_TEST`, `variables.VM_SMOKE`, or `variables.block(...)`."
    )


def test_variables_refuses_a_name_that_is_not_approved() -> None:
    """The indirection has to fail loudly, or it is just a longer literal."""
    import pytest

    with pytest.raises(KeyError, match="not an approved public recipe"):
        variables.recipe("smoke")


def test_every_approved_recipe_exists_in_the_justfile() -> None:
    """The ledger and the justfile cannot drift apart silently.

    `block` raises rather than returning empty, so a recipe that was approved
    and never written -- or renamed in one file only -- is a failure here
    instead of a contract that quietly asserts nothing about an empty string.
    """
    for name in variables.PUBLIC_RECIPES:
        assert variables.block(name) is not None
