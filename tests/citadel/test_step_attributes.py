"""Citadel guard: a step says what it is, and the ones that do not are counted.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one guards against a property being *inferred* rather than
declared -- the mistake that makes a contract depend on a naming convention.
"""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.execution import Arch, Kind, Needs, Speed

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(PROJECT_ROOT)
LEDGER = CONFIG.boundary.step_attributes
GATE = PROJECT_ROOT / "src" / "capsem" / "gate"

#: The attributes that together make a step declared. `needs`, `arch` and
#: `concurrency` have honest defaults -- no capability, any architecture, one
#: worker -- but there is no honest default for what a step *is* or which lane
#: it belongs in, so those two are what the migration counts.
REQUIRED = ("kind", "speed")

STEP_ATTRIBUTE_RATIONALE = """\
Every step declares what it is; nothing infers it from the label.

The plan is a DAG, and almost every question worth asking about the release is
a question about that graph: may this run in the fast lane, is this reachable
without the network, do these two ever overlap, is the documented order a valid
topological sort. None of those can be answered by a string.

They were answered by strings anyway. A contract matched `fast.` against a
label to decide a step was cheap. A documentation check mapped stage titles to
label prefixes. `test_release_doctor_contract.py` greps YAML for the
serialisation of an edge set, which is why reordering a `needs:` list -- the
same list -- once failed four contracts while changing nothing GitHub acts on.
Renaming a step could silently change what was being checked, and nothing
would fail.

So `Step` carries the answer:

  kind         lint, static test, compile, unit test, capsem, e2e, package,
               publish -- what the work *is*
  needs        the capability set: network, disk, docker, vm, kvm, signing.
               Hermeticity is derived from it, never declared, because a step
               must not be able to claim a property its inputs contradict
  arch         host, x86_64, arm64, any -- so an edge across architectures is
               a detectable error rather than a convention
  speed        which lane it belongs in, categorically. Not a duration:
               `[runlog.timing_regression]` says no guessed number of seconds
               belongs in config, and that holds here
  concurrency  how much machine it takes, which `contends` does not say

`[boundary.step_attributes]` is a migration ledger with a destination, not an
exemption list. The count may only fall. When it reaches zero the defaults come
off `Step` and these become required arguments -- the point at which a new step
cannot be added without saying what it is.

See config/gate.toml [boundary.step_attributes] and src/capsem/gate/execution.py.
"""


def _constructions(module: Path) -> list[ast.Call]:
    """Every `step(...)` call in a module.

    Parsed rather than grepped: `step` appears in prose, in `Step` type
    annotations and in `plan.add(step)` where the name is a variable, and a
    guard that counts those is counting the wrong thing.
    """
    tree = ast.parse(module.read_text(encoding="utf-8"), filename=str(module))
    return [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Name)
        and node.func.id == "step"
    ]


def _undeclared() -> dict[str, int]:
    """Undeclared construction sites per module, as the ledger records them."""
    counted: dict[str, int] = {}
    for module in sorted(GATE.glob("*.py")):
        missing = sum(
            1
            for call in _constructions(module)
            if not set(REQUIRED) <= {keyword.arg for keyword in call.keywords}
        )
        if missing:
            counted[module.name] = missing
    return counted


def test_the_ledger_has_a_destination() -> None:
    """A ledger with no target is an exemption list wearing a number."""
    assert LEDGER.max_undeclared == 0, (
        STEP_ATTRIBUTE_RATIONALE
        + f"\nmax_undeclared is {LEDGER.max_undeclared}; the migration ends at zero"
    )


def test_no_module_gains_undeclared_steps() -> None:
    """Exact counts, so a module may shrink but never grow.

    Exact rather than a total: a total lets one module improve while another
    regresses and reports nothing, which is how a ratchet stops ratcheting.
    """
    actual = _undeclared()
    grown = {
        name: f"{LEDGER.undeclared_by_module.get(name, 0)} -> {count}"
        for name, count in actual.items()
        if count > LEDGER.undeclared_by_module.get(name, 0)
    }
    assert not grown, (
        STEP_ATTRIBUTE_RATIONALE
        + f"\nmodules gained undeclared steps: {grown}\n"
        "Declare kind= and speed= on the new step rather than widening the ledger."
    )


def test_the_ledger_is_not_stale() -> None:
    """A module that has been migrated updates its entry in the same change.

    An inventory that drifts above the tree has stopped ratcheting: it permits
    a regression back up to a number nobody is paying any more.
    """
    actual = _undeclared()
    stale = {
        name: f"recorded {recorded}, actually {actual.get(name, 0)}"
        for name, recorded in LEDGER.undeclared_by_module.items()
        if actual.get(name, 0) < recorded
    }
    assert not stale, (
        STEP_ATTRIBUTE_RATIONALE
        + f"\nledger entries are above the tree: {stale}\n"
        "Lower or remove the entry in the change that declared the steps."
    )


def test_a_declared_step_uses_the_closed_vocabularies() -> None:
    """The enums are the vocabulary; a bare string would defeat them.

    Checked on the values rather than the annotations, because `StrEnum`
    members *are* strings and a plain string would satisfy every type check
    while being outside the vocabulary.
    """
    for name, member in (("Kind", Kind), ("Speed", Speed), ("Arch", Arch), ("Needs", Needs)):
        assert len(set(member)) == len(list(member)), f"{name} has duplicate values"
    assert Kind.UNDECLARED in Kind and Speed.UNDECLARED in Speed, (
        "the migration sentinels must exist until the ledger reaches zero"
    )


# -- adversarial: the counter has to see what it claims to ------------------


@pytest.mark.parametrize(
    ("source", "counted"),
    [
        ("step('a', Run([]))", 1),
        ("step('a', Run([]), kind=Kind.LINT)", 1),
        ("step('a', Run([]), speed=Speed.FAST)", 1),
        ("step('a', Run([]), kind=Kind.LINT, speed=Speed.FAST)", 0),
        ("plan.add(step('a', Run([]), kind=Kind.LINT, speed=Speed.FAST))", 0),
        ("x: Step = other_step", 0),
        ("'step( in a string'", 0),
    ],
)
def test_the_counter_sees_only_real_constructions(
    source: str, counted: int, tmp_path: Path
) -> None:
    module = tmp_path / "sample.py"
    module.write_text(source, encoding="utf-8")
    missing = sum(
        1
        for call in _constructions(module)
        if not set(REQUIRED) <= {keyword.arg for keyword in call.keywords}
    )
    assert missing == counted, f"{source!r} counted {missing}, expected {counted}"
