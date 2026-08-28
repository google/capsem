"""Reject competing release specifications and stale authority routing."""

from __future__ import annotations

import hashlib
import re
import subprocess
from collections.abc import Mapping, Sequence
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CANONICAL_TITLE = "# Capsem Binary, Profile, Manifest, and Channel Release Specification"
OLD_AUTHORITY = "tmp/release-spec.md"
ROUTING_SURFACES = ("AGENTS.md", "skills/release-process/")
AUTHORITY_CLAIMS = (
    re.compile(r"\bnormative contract\b", re.IGNORECASE),
    re.compile(r"\bgoverning contract\b", re.IGNORECASE),
    re.compile(r"\bCapsem has exactly two release(?:-facing)?(?: Just)? commands\b", re.IGNORECASE),
    re.compile(r"\b(?:MUST|MUST NOT|REQUIRED|SHOULD|SHOULD NOT|MAY)\b"),
)

RELEASE_AUTHORITY_RATIONALE = """\
Release safety cannot have two prose authorities. Root RELEASE.md owns every
normative invariant; AGENTS and skills may link to it and retain operational
lessons, while tests provide executable evidence. A temporary stale reference
may survive only as exact content-hashed debt, such as the Rust-owned caller
held until the user confirms the concurrent crate work is merged and main is
pulled.
"""


def _is_routing_surface(path: str) -> bool:
    return any(path == prefix or path.startswith(prefix) for prefix in ROUTING_SURFACES)


def _digest(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def _violations(
    tracked: Sequence[str],
    sources: Mapping[str, str],
    debt: Mapping[str, str] | None = None,
) -> list[str]:
    tracked_set = set(tracked)
    debt = debt or {}
    problems: list[str] = []
    if "RELEASE.md" not in tracked_set:
        problems.append("root RELEASE.md is not tracked")
    elif not sources.get("RELEASE.md", "").startswith(CANONICAL_TITLE):
        problems.append("root RELEASE.md is not the complete canonical specification")
    if OLD_AUTHORITY in tracked_set:
        problems.append(f"obsolete normative source is still tracked: {OLD_AUTHORITY}")

    stale_paths = {
        path for path, text in sources.items() if OLD_AUTHORITY in text and path != OLD_AUTHORITY
    }
    for path in sorted(stale_paths):
        if debt.get(path) != _digest(sources[path]):
            problems.append(f"stale release-authority reference: {path}")
    for path in sorted(set(debt) - stale_paths):
        problems.append(f"stale release-authority debt entry: {path}")

    for path, text in sorted(sources.items()):
        if path == "RELEASE.md" or not _is_routing_surface(path):
            continue
        claims = [pattern.pattern for pattern in AUTHORITY_CLAIMS if pattern.search(text)]
        if claims:
            problems.append(f"duplicate normative release authority in {path}: {claims}")
    return sorted(problems)


def _tracked_sources() -> tuple[list[str], dict[str, str]]:
    output = subprocess.run(
        ("git", "ls-files", "-s", "-z"), cwd=ROOT, check=True, capture_output=True
    ).stdout
    tracked: list[str] = []
    sources: dict[str, str] = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        metadata, raw_path = record.split(b"\t", 1)
        mode = metadata.split(b" ", 1)[0]
        path = raw_path.decode("utf-8")
        tracked.append(path)
        if mode == b"120000":
            continue
        raw = (ROOT / path).read_bytes()
        if b"\0" in raw:
            continue
        try:
            sources[path] = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
    return tracked, sources


def test_missing_root_authority_fails_closed() -> None:
    assert _violations([], {}) == [
        "root RELEASE.md is not tracked"
    ], RELEASE_AUTHORITY_RATIONALE


def test_old_normative_source_and_reference_are_rejected() -> None:
    tracked = ["RELEASE.md", OLD_AUTHORITY, "AGENTS.md"]
    sources = {
        "RELEASE.md": CANONICAL_TITLE,
        OLD_AUTHORITY: "# old",
        "AGENTS.md": f"Read `{OLD_AUTHORITY}`.",
    }
    problems = _violations(tracked, sources)
    assert f"obsolete normative source is still tracked: {OLD_AUTHORITY}" in problems
    assert "stale release-authority reference: AGENTS.md" in problems


@pytest.mark.parametrize(
    "claim",
    [
        "The normative contract is elsewhere.",
        "This is the governing contract.",
        "Capsem has exactly two release-facing Just commands.",
        "A profile release MUST NOT build binaries.",
    ],
)
def test_agent_and_skill_normative_duplicates_are_rejected(claim: str) -> None:
    problems = _violations(
        ["RELEASE.md", "skills/release-process/SKILL.md"],
        {
            "RELEASE.md": CANONICAL_TITLE,
            "skills/release-process/SKILL.md": claim,
        },
    )
    assert any("duplicate normative release authority" in problem for problem in problems)


def test_operational_link_and_routing_are_allowed() -> None:
    tracked = ["RELEASE.md", "AGENTS.md", "skills/release-process/SKILL.md"]
    sources = {
        "RELEASE.md": CANONICAL_TITLE,
        "AGENTS.md": "Release authority: [RELEASE.md](RELEASE.md).",
        "skills/release-process/SKILL.md": (
            "Read [RELEASE.md](../../RELEASE.md), then use the signing reference."
        ),
    }
    assert not _violations(tracked, sources), RELEASE_AUTHORITY_RATIONALE


def test_exact_hashed_debt_is_allowed_and_reconciles_both_ways() -> None:
    path = "crates/example/src/tests.rs"
    text = f"// Update `{OLD_AUTHORITY}` after the held Rust merge."
    tracked = ["RELEASE.md", path]
    sources = {"RELEASE.md": CANONICAL_TITLE, path: text}
    debt = {path: _digest(text)}
    assert not _violations(tracked, sources, debt), RELEASE_AUTHORITY_RATIONALE
    assert f"stale release-authority reference: {path}" in _violations(
        tracked, sources, {path: "wrong"}
    )
    assert f"stale release-authority debt entry: {path}" in _violations(
        ["RELEASE.md"], {"RELEASE.md": CANONICAL_TITLE}, debt
    )
