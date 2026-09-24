"""Citadel guard: only the logger opens the session body archive.

`session.bodies` is the second half of the session ledger, and it is only
readable together with the SQLite index that names its blocks. A route, UI or
benchmark that opens the file itself would be reimplementing block lookup,
bounds checking and hash verification beside the one place that already owns
them -- and would read bodies the DB handle could not account for.

The rule is the same one the DB boundary states: callers own query intent, the
logger owns execution and storage. See AGENTS.md and skills/dev-testing.
"""

from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
CRATES_DIR = PROJECT_ROOT / "crates"

BODY_ARCHIVE_RATIONALE = (
    "Body archive opened outside capsem-logger; use DbHandle::read_body / "
    "read_bodies or GET /vms/{id}/bodies/{event_id}."
)

# The archive's own crate writes and reads the file; the logger owns the index
# that says where in it anything lives. Nothing else may name either.
ARCHIVE_OWNERS = ("capsem-archive", "capsem-logger")

NEEDLES: tuple[str, ...] = (
    "BodyLogReader::open",
    "BodyLogWriter::open",
    "session.bodies",
)


def owns_the_archive(path: Path) -> bool:
    """True for a file inside one of the two crates that own the archive."""
    parts = path.parts
    if "crates" not in parts:
        return False
    crate = parts.index("crates") + 1
    return crate < len(parts) and parts[crate] in ARCHIVE_OWNERS


def archive_violations(path: Path, text: str) -> list[str]:
    """Pure function of (path, text): every forbidden needle in a non-owner.

    Pure so the adversarial case below can hand it a file whose content is the
    violation, rather than trusting that a repository with no violation in it
    proves the predicate can find one.
    """
    try:
        relative = path.relative_to(PROJECT_ROOT)
    except ValueError:
        relative = path
    if owns_the_archive(path):
        return []
    return [f"{relative} contains `{needle}`" for needle in NEEDLES if needle in text]


def test_only_the_logger_and_the_archive_open_the_body_archive() -> None:
    violations: list[str] = []
    for path in sorted(CRATES_DIR.rglob("*.rs")):
        violations.extend(archive_violations(path, path.read_text()))

    assert not violations, BODY_ARCHIVE_RATIONALE + "\n" + "\n".join(violations)


def test_the_guard_flags_a_file_that_opens_the_archive(tmp_path: Path) -> None:
    """A guard that cannot fail is not a guard.

    The repository is expected to be clean, so the only way to know this one
    still detects anything is to hand it an offender.
    """
    offender = tmp_path / "crates" / "capsem-service" / "src" / "bodies_route.rs"
    offender.parent.mkdir(parents=True)
    source = 'let reader = BodyLogReader::open(&dir.join("session.bodies"))?;'
    offender.write_text(source)

    found = archive_violations(offender, source)
    assert len(found) == 2, f"the predicate must flag both needles: {found}"

    owner = tmp_path / "crates" / "capsem-logger" / "src" / "db" / "bodies.rs"
    owner.parent.mkdir(parents=True)
    assert archive_violations(owner, source) == [], "the owning crates are not offenders"
