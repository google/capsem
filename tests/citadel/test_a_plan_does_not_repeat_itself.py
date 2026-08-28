"""Citadel guard: one plan should not do the same work three times.

`generate-settings.sh` takes about seventy-five seconds and the candidate plan
runs it three times -- `fast.audit.generated-settings`,
`static.audit.generated-settings`, `functional.audit.generated-settings` -- for
the same generated file, in the same prefix, in one run. Nobody chose that. Each
module owns its prerequisites so it can run alone, which is right, and nothing
noticed what happens when they are composed.

The existing guards all ask about *shape*: what may run beside what, what a step
must follow, which modules may touch the machine, whether every step declares
its attributes. None of them asks whether two steps are the same work, so a
plan could repeat an expensive action indefinitely and stay green. It did: a
twenty-one minute "fast" gate, of which several minutes were the same script
run again.

Deliberate repetition exists and is fine -- `pnpm install --frozen-lockfile` is
idempotent, cheap against a warm tree, and cheaper than a module whose
independence depends on having been run after another one. So this is a ledger
rather than a ban: a repeated action is declared with its reason, and anything
undeclared is work nobody decided to pay for twice.
"""

from __future__ import annotations

from collections import defaultdict
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate import host
from helpers.gate import gate_plan

ROOT = Path(__file__).resolve().parents[2]


#: The action kinds that spend real time on a real command. A `Call` renders its
#: own description rather than its arguments, so two of them targeting different
#: architectures read identically while doing entirely different work -- and a
#: guard that counted those would report eight repetitions that are not.
COMMANDS = frozenset({"script", "run"})


def _active_exclusions():
    entries = gate_config.load(ROOT).boundary.repeated_actions
    return tuple(
        entry for entry in entries if not entry.platforms or host.system() in entry.platforms
    )


def _repeated(name: str) -> dict[str, list[str]]:
    """Every command a plan issues more than once, and the steps that issue it."""
    seen: dict[str, list[str]] = defaultdict(list)
    for step in gate_plan(name).steps:
        for action in step.actions:
            if type(action).name in COMMANDS:
                seen[action.render()].append(step.label)
    return {rendered: labels for rendered, labels in seen.items() if len(labels) > 1}


def test_the_candidate_plan_repeats_only_what_it_declares() -> None:
    """The whole gate, which is where composition makes repetition invisible."""
    allowed = {exclusion.subject for exclusion in _active_exclusions()}

    undeclared = {
        rendered: labels
        for rendered, labels in _repeated("candidate").items()
        if not any(marker in rendered for marker in allowed)
    }

    assert not undeclared, (
        "these run the same command more than once in a single plan, and "
        "nothing decided to pay for it twice:\n  "
        + "\n  ".join(f"{labels}: {rendered[:120]}" for rendered, labels in undeclared.items())
    )


def test_every_declared_repetition_still_happens() -> None:
    """A ledger that outlives what it excuses stops being a ledger.

    An entry here is a repetition somebody looked at and accepted. Once the
    repetition is gone the entry is a claim about a plan that no longer exists,
    and the next reader believes it.
    """
    rendered = " ".join(_repeated("candidate"))

    stale = [
        exclusion.subject
        for exclusion in _active_exclusions()
        if exclusion.subject not in rendered
    ]
    assert not stale, f"{stale} no longer repeats in the candidate plan; drop the entry"


def test_platform_scoped_repetitions_are_active_only_on_their_host(monkeypatch) -> None:
    entries = gate_config.load(ROOT).boundary.repeated_actions
    scoped = tuple(entry for entry in entries if entry.platforms)
    assert scoped, "this guard has no platform-scoped ledger entry to prove"

    for system in ("Darwin", "Linux"):
        monkeypatch.setattr(host, "system", lambda system=system: system)
        active = _active_exclusions()
        assert all((system in entry.platforms) == (entry in active) for entry in scoped)
