"""Citadel guard: one exec deadline, spelled the same in every client.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is a number copied into four languages.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]

#: Where the service and the gateway define what a client has to wait for.
SERVICE_CEILING = ("crates/capsem-api/src/execution.rs", r"MAX_EXEC_TIMEOUT_SECS: u64 = (.+?);")
GATEWAY_BUDGET = ("crates/capsem-gateway/src/proxy.rs", r"REQUEST_TIMEOUT: Duration = Duration::from_secs\((\d+)\)")
#: Each client's copy of those two numbers, in seconds.
CLIENT_CEILINGS = (
    ("sdk/python/capsem/execution.py", r"EXEC_TIMEOUT_CEILING_SECS = (.+)"),
    ("sdk/typescript/src/execution.ts", r"EXEC_TIMEOUT_CEILING_SECS = (.+?);"),
    ("sdk/rust/src/client.rs", r"EXEC_TIMEOUT_CEILING: Duration = Duration::from_secs\((.+?)\)"),
)
CLIENT_BUDGETS = (
    ("sdk/python/capsem/execution.py", r"GATEWAY_REQUEST_BUDGET_SECS = (.+)"),
    ("sdk/typescript/src/execution.ts", r"GATEWAY_REQUEST_BUDGET_SECS = (.+?);"),
    ("sdk/rust/src/client.rs", r"GATEWAY_REQUEST_BUDGET: Duration = Duration::from_secs\((.+?)\)"),
)
#: The desktop UI waits at least as long as the gateway does.
WEB_BUDGET = ("web/app/src/lib/gateway-sdk.ts", r"GATEWAY_REQUEST_BUDGET_MS = (.+?);")

DEADLINE_RATIONALE = """\
Exec and run answer only when the command finishes.

The service bounds that by the request's timeout_secs, defaulting to
MAX_EXEC_TIMEOUT_SECS, and the gateway waits that ceiling plus its ordinary
REQUEST_TIMEOUT. Clients whose only deadline was their 30 s transport default
aborted long commands that kept running server-side, and a retrying agent
started a second build.

Every SDK now stretches its per-call deadline to the same two numbers, so the
numbers must be the same numbers. Change the service or gateway value and this
guard names every client that still holds the old one.
"""


def seconds(spelling: str) -> int:
    """Evaluate a simple integer constant: `3600`, `60 * 60`, `120`."""
    cleaned = spelling.strip().rstrip(";").replace("_", "")
    if not re.fullmatch(r"\d+(\s*\*\s*\d+)*", cleaned):
        raise ValueError(f"unsupported constant spelling: {spelling!r}")
    total = 1
    for part in cleaned.split("*"):
        total *= int(part)
    return total


def constant(source: tuple[str, str]) -> int:
    path, pattern = source
    text = (PROJECT_ROOT / path).read_text()
    match = re.search(pattern, text)
    assert match, f"{path} no longer defines {pattern!r}.\n\n{DEADLINE_RATIONALE}"
    return seconds(match.group(1))


def test_every_client_waits_for_the_service_exec_ceiling() -> None:
    expected = constant(SERVICE_CEILING)
    drifted = {path: constant((path, pattern)) for path, pattern in CLIENT_CEILINGS}
    assert set(drifted.values()) == {expected}, f"service ceiling is {expected}s: {drifted}.\n\n{DEADLINE_RATIONALE}"


def test_every_client_adds_the_gateway_request_budget() -> None:
    expected = constant(GATEWAY_BUDGET)
    drifted = {path: constant((path, pattern)) for path, pattern in CLIENT_BUDGETS}
    assert set(drifted.values()) == {expected}, f"gateway budget is {expected}s: {drifted}.\n\n{DEADLINE_RATIONALE}"


def test_the_desktop_ui_waits_at_least_as_long_as_the_gateway() -> None:
    milliseconds = constant(WEB_BUDGET)
    assert milliseconds >= constant(GATEWAY_BUDGET) * 1000, DEADLINE_RATIONALE


@pytest.mark.parametrize(
    ("spelling", "expected"),
    [("3600", 3600), ("60 * 60", 3600), ("120", 120), ("125_000", 125000), ("60*60", 3600)],
)
def test_constant_spellings_are_read_as_seconds(spelling: str, expected: int) -> None:
    assert seconds(spelling) == expected


@pytest.mark.parametrize("spelling", ["3600.5", "TIMEOUT", "60 + 60", "", "0x10"])
def test_unsupported_spellings_fail_loudly_instead_of_passing(spelling: str) -> None:
    with pytest.raises(ValueError):
        seconds(spelling)
