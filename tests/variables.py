"""The names the build system is spelled with, read from their one source.

Renaming `smoke` to `fast-test` and `vm-smoke` broke five contracts in four
files, none of which was testing behaviour. They asserted on the *spelling* of
a recipe -- "the block called `smoke:` contains these lines" -- so a rename
that changed no behaviour at all failed the build, and a behaviour change that
kept the name would have passed. That is the wrong way round in both
directions.

This is not a second source of truth. Every value here is derived at import
from the file that already owns it:

  * public recipe names come from `config/public-surface.toml`, which is the
    approval ledger the surface contract already checks;
  * gate command names come from the gate's own command registry.

So there is still exactly one place to change a name, and it is the same place
as before. What this adds is a way for a test to *ask* rather than to guess,
and `test_surface_names_are_not_hardcoded.py` makes asking the only option.

A recipe name is not the same thing as the gate command it dispatches to.
`vm-smoke` runs `capsem-gate smoke`; conflating them is exactly the mistake
that produced a `KeyError: 'vm-smoke'` while this was being written, so the
two are separate mappings here.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]

_SURFACE = tomllib.loads(
    (PROJECT_ROOT / "config" / "public-surface.toml").read_text(encoding="utf-8")
)

#: Every approved public `just` recipe, in the order the ledger lists them.
PUBLIC_RECIPES: tuple[str, ...] = tuple(_SURFACE["just"]["approved"])


def recipe(name: str) -> str:
    """A public recipe name, refused if it is not on the approved surface.

    The point of the indirection: a typo or a stale name fails here, at import,
    naming the surface -- instead of somewhere downstream as a `StopIteration`
    from a recipe lookup that found nothing.
    """
    if name not in PUBLIC_RECIPES:
        raise KeyError(
            f"{name!r} is not an approved public recipe. The surface is "
            f"{list(PUBLIC_RECIPES)}; change config/public-surface.toml first, "
            "because that is the API approval and this only reads it."
        )
    return name


#: The fast gate, run alone. Identical to the `_test-fast` module `test` and
#: both release lanes run, which is the whole reason it is public.
FAST_TEST = recipe("fast-test")

#: A short VM round-trip: boot, exercise, tear down. Runtime liveness only.
VM_SMOKE = recipe("vm-smoke")

#: The complete local proof, and the only release qualification.
TEST = recipe("test")

#: What each public recipe dispatches to inside `capsem-gate`, where it does.
#: Deliberately partial: most recipes are not one gate command, and inventing
#: an entry for them would assert a structure that is not there.
GATE_COMMAND_FOR_RECIPE: dict[str, str] = {
    VM_SMOKE: "smoke",
    TEST: "candidate",
}


def header(name: str) -> str:
    """The recipe's declaration line, which is where its dependencies live.

    `vm-smoke: _prepared-runtime` states an ordering that never appears in the
    body, so a contract reading only `block` concludes the preparation is
    missing when it is simply declared the other way.
    """
    lines = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    pattern = re.compile(rf"^{re.escape(recipe(name))}(\s|:)")
    return next(line for line in lines if pattern.match(line))


def block(name: str) -> str:
    """The recipe's body, found by its approved name rather than a literal."""
    lines = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    # `build profile="debug":` is the same recipe as `build:` -- a header is
    # the name, then optional parameters, then the colon. Matching only
    # `name:` silently found nothing for every parameterised recipe.
    header = re.compile(rf"^{re.escape(recipe(name))}(\s|:)")
    start = next(
        (index for index, line in enumerate(lines) if header.match(line)),
        None,
    )
    if start is None:
        raise AssertionError(
            f"the approved recipe {name!r} is not in the justfile. The surface "
            "ledger and the justfile disagree, which the public-surface "
            "contract would also catch."
        )
    body: list[str] = []
    for line in lines[start + 1 :]:
        if line and not line[0].isspace():
            break
        body.append(line)
    return "\n".join(body)
