"""Citadel guard: a suite that stops at one failure hides the other four.

`--maxfail=1` was the default for every suite that did not opt out, and the cost
is measurable rather than theoretical. The one phase where it is already off --
release contracts -- reported four failures in a single eleven-minute run, and
all four were fixed in one pass. Had it stopped at the first, the same four
would have cost four runs and three quarters of an hour.

Removing the bound entirely is the other mistake. When a VM or a service is
broken, every remaining test in a slow suite fails the same way, and an hour is
spent restating one fact. So the bound is a budget rather than a switch: enough
failures to show a pattern, few enough to stop a cascade.
"""

from __future__ import annotations

from pathlib import Path

from capsem.gate import config as gate_config

ROOT = Path(__file__).resolve().parents[2]


def test_a_bounded_suite_reports_a_budget_and_not_a_single_failure() -> None:
    """One failure is a sighting; several are a diagnosis."""
    settings = gate_config.load(ROOT).suites.pytest

    budget = settings.stop_at_first
    assert budget.startswith("--maxfail="), budget
    allowed = int(budget.removeprefix("--maxfail="))
    assert allowed > 1, (
        "a suite that stops at the first failure turns one run into one fact, "
        "and the next fact costs another run"
    )
    assert allowed <= 10, (
        f"a budget of {allowed} is close enough to unbounded that a broken VM "
        "spends the whole suite restating one failure"
    )
