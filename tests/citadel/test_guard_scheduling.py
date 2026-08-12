"""Citadel guard for when the Citadel itself runs.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is about the Citadel: a guard scheduled after the expensive
work is a guard that reports the mistake too late to have prevented anything.
"""

from __future__ import annotations

from helpers.gate import gate_plan

GUARD_SCHEDULING_RATIONALE = """\
Citadel guards must run in the fast phase.

Every guard in tests/citadel/ reads source and asserts on it. None needs a
built artifact, a VM, or a running daemon, and the whole suite answers in
about a fifth of a second.

They were reachable only through the broad suite's `root`, which carries
`require_artifacts` and runs after the entire asset build -- so a DB-boundary
violation was reported once the VMs were already up, roughly forty minutes
after the source that caused it was read. A guard whose own docstring says it
exists to "fail before a hidden route cache, direct SQLite open, or
compatibility fallback can ship green" cannot be scheduled behind the thing it
is supposed to precede.

Two rules follow, and this test holds both:

  1. `fast.citadel` exists and is in the fast phase, so it answers alongside
     Ruff rather than behind the contract suite.
  2. `contracts.release` -- the nine-minute pytest run -- waits on the fast
     phase, so no expensive step can start while a guard is still unproven.

If you are moving this suite, move it earlier, never later. And keep exactly
one owner: `tests/citadel` is in `broad_ignores` because a guard collected by
two suites is a guard whose failures nobody owns.

See config/gate.toml [suites.pytest] and skills/dev-gate/SKILL.md.
"""

CITADEL_STEP = "fast.citadel"
CONTRACTS_STEP = "contracts.release"
FAST_PHASE = "fast."


def test_the_citadel_runs_in_the_fast_phase_before_the_contract_suite() -> None:
    plan = gate_plan("candidate")
    labels = set(plan.labels)

    problems: list[str] = []
    if CITADEL_STEP not in labels:
        problems.append(f"{CITADEL_STEP} is not in the candidate plan at all")
    elif not CITADEL_STEP.startswith(FAST_PHASE):
        problems.append(f"{CITADEL_STEP} is no longer in the fast phase")

    # An edge, not a position: the contract suite must transitively wait on the
    # guards rather than merely happen to be written after them.
    if CONTRACTS_STEP in labels and CITADEL_STEP in labels:
        waited_on = _ancestors(plan, CONTRACTS_STEP)
        if CITADEL_STEP not in waited_on:
            problems.append(
                f"{CONTRACTS_STEP} does not wait on {CITADEL_STEP}; "
                "the guards can still be running when the expensive work starts"
            )

    assert not problems, GUARD_SCHEDULING_RATIONALE + "\n" + "\n".join(problems)


def _ancestors(plan, label: str) -> set[str]:
    """Every step `label` transitively waits for."""
    seen: set[str] = set()
    pending = list(plan.after_of(label))
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        pending.extend(plan.after_of(current))
    return seen
