"""Citadel guard: profiles are edited only inside the policy mutation boundary.

A policy edit is load, modify, save, cache refresh, active-profile
materialization and VM acknowledgement. Each step used to take its own short
lock, so two edits interleaved: B loaded before A saved and wrote A's rule
away, or B re-published a session while A waited for that VM's
acknowledgement (google/capsem#202, owned by #229).

capsem-service now serializes the sequence through `PolicyMutation`. Recording
and pushing a mutation take it as an argument, so those cannot compile outside
it; this guard closes the remaining door, which is loading an editable profile
without it. Outside `PolicyMutation::profile`, a loaded or cached profile is
only ever bound immutably, for reading.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SERVICE_SRC = Path("crates/capsem-service/src")
BOUNDARY = SERVICE_SRC / "policy_mutation.rs"

RATIONALE = """\
Load a profile to edit through PolicyMutation::profile, inside
apply_profile_mutation or a held state.policy_mutation.begin(). Binding a
loaded or cached profile mutably anywhere else lets a route read-modify-write
policy outside the boundary and lose a concurrent edit.
"""

MUTABLE_LOAD = re.compile(r"let\s+mut\s+\w+\s*=\s*(?:\w+::)*(?:cached_)?profile_for_route\s*\(")


def _production_sources(root: Path) -> list[Path]:
    return sorted(
        path
        for path in (root / SERVICE_SRC).rglob("*.rs")
        if "tests" not in path.relative_to(root / SERVICE_SRC).parts[:-1]
        and path.name != "tests.rs"
    )


def unserialized_profile_edits(root: Path = PROJECT_ROOT) -> list[str]:
    found: list[str] = []
    for path in _production_sources(root):
        relative = path.relative_to(root)
        if relative == BOUNDARY:
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            code = line.split("//", 1)[0]
            if MUTABLE_LOAD.search(code):
                found.append(f"{relative}:{number}: {code.strip()}")
    return found


def test_profiles_are_edited_only_inside_the_policy_mutation_boundary() -> None:
    found = unserialized_profile_edits()
    assert not found, RATIONALE + "\n" + "\n".join(found)


def test_guard_detects_an_unserialized_edit(tmp_path: Path) -> None:
    source = tmp_path / SERVICE_SRC / "routes.rs"
    source.parent.mkdir(parents=True)
    (tmp_path / BOUNDARY).write_text("fn f() { profile_routes::profile_for_route(id) }\n", encoding="utf-8")
    source.write_text(
        "fn a() { let mut profile = profile_for_route(id)?; }\n"
        "fn b() { let mut p = cached_profile_for_route(&s, id)?; }\n"
        "fn c() { let p = profile_for_route(id)?; }\n",
        encoding="utf-8",
    )
    assert [line.split(": ", 1)[0] for line in unserialized_profile_edits(tmp_path)] == [
        f"{SERVICE_SRC / 'routes.rs'}:1",
        f"{SERVICE_SRC / 'routes.rs'}:2",
    ]
