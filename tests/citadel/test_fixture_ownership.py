"""Require every legacy root fixture to have one explicit disposition and owner."""

from __future__ import annotations

import hashlib
import re
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

ROOT = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("fixture_ownership.toml")
SELF = "tests/citadel/test_fixture_ownership.py"
POLICY_PATH = "tests/citadel/fixture_ownership.toml"

RATIONALE = """\
Fixtures are test inputs, never product configuration or an anonymous shared
data lake. A retained fixture names its exact executable consumer; a deletion
has no caller; and every temporary root read is exact migration debt. Empty or
partial inventories fail closed so a move cannot silently abandon test bytes.
"""


@dataclass(frozen=True)
class Companion:
    """A second file of the same fixture, moving with it."""

    path: str
    sha256: str


@dataclass(frozen=True)
class Fixture:
    source: str
    target: str
    disposition: str
    owner: str
    item: str
    consumers: tuple[str, ...]
    symbols: tuple[str, ...]
    sha256: str
    companions: tuple[Companion, ...]


@dataclass(frozen=True)
class Audit:
    tracked_data: tuple[str, ...]
    tracked_all: tuple[str, ...]
    ignored_data: tuple[str, ...]
    untracked_data: tuple[str, ...]
    root_reads: tuple[str, ...]
    consumers: dict[str, tuple[str, ...]]
    orphan_references: dict[str, tuple[str, ...]]
    digests: dict[str, str]
    binary: tuple[str, ...]


def _git(*args: str, root: Path = ROOT) -> list[str]:
    result = subprocess.run(
        ("git", *args),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.splitlines()


def _fixtures(policy: dict[str, Any]) -> list[Fixture]:
    rows = policy.get("fixture", [])
    if not isinstance(rows, list):
        return []
    return [
        Fixture(
            source=row.get("source", ""),
            target=row.get("target", ""),
            disposition=row.get("disposition", ""),
            owner=row.get("owner", ""),
            item=row.get("item", ""),
            consumers=(
                tuple(row.get("consumers", []))
                if isinstance(row.get("consumers"), list)
                else (row.get("consumer", ""),)
            ),
            symbols=tuple(row.get("symbols", [])),
            sha256=row.get("sha256", ""),
            companions=tuple(
                Companion(path=entry.get("path", ""), sha256=entry.get("sha256", ""))
                for entry in row.get("companion", [])
                if isinstance(entry, dict)
            ),
        )
        for row in rows
        if isinstance(row, dict)
    ]


def _digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _is_binary(path: Path) -> bool:
    with path.open("rb") as handle:
        return b"\0" in handle.read(8192)


def _function_map(path: Path) -> dict[int, str]:
    current = "<module>"
    mapping: dict[int, str] = {}
    for number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        match = re.match(r"\s*(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)", line)
        if match:
            current = match.group(1)
        mapping[number] = current
    return mapping


def _consumer_symbols(fixture: Fixture) -> tuple[str, ...]:
    if not fixture.consumers or any(not consumer for consumer in fixture.consumers):
        return ()
    name = Path(fixture.source).name
    symbols: list[str] = []
    for consumer in fixture.consumers:
        path = ROOT / consumer
        if not path.is_file():
            return ()
        functions = _function_map(path)
        for number, line in enumerate(
            path.read_text(encoding="utf-8").splitlines(), start=1
        ):
            if name.endswith(".html") and f'load_fixture("{name}")' in line:
                symbols.append(functions[number])
            elif name == "test.db" and "fixture_reader()" in line:
                symbol = functions[number]
                if symbol != "fixture_reader":
                    symbols.append(symbol)
    return tuple(sorted(symbols))


def _text_sources(paths: list[str]) -> dict[str, str]:
    sources: dict[str, str] = {}
    for path in paths:
        if path in {SELF, POLICY_PATH} or path.startswith(
            ("data/", "sprints/", "tmp/")
        ):
            continue
        candidate = ROOT / path
        if candidate.is_symlink() or not candidate.is_file():
            continue
        raw = candidate.read_bytes()
        if b"\0" in raw:
            continue
        try:
            sources[path] = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
    return sources


def _audit(fixtures: list[Fixture]) -> Audit:
    tracked_all = _git("ls-files")
    tracked_data = _git("ls-files", "data")
    ignored_data = _git(
        "ls-files", "--others", "--ignored", "--exclude-standard", "data"
    )
    untracked_data = _git("ls-files", "--others", "--exclude-standard", "data")
    sources = _text_sources(tracked_all)
    consumers = {
        fixture.source: _consumer_symbols(fixture)
        for fixture in fixtures
        if fixture.disposition == "retain"
    }
    root_reads = tuple(
        sorted(
            f"{path}:{line.strip()}"
            for path, text in sources.items()
            if not path.startswith("tests/citadel/")
            and (
                Path(path).suffix in {".py", ".rs", ".sh"}
                or Path(path).name == "justfile"
            )
            for line in text.splitlines()
            if "data/fixtures" in line
        )
    )
    digests: dict[str, str] = {}
    binary: list[str] = []
    for fixture in fixtures:
        for name in (fixture.source, *(c.path for c in fixture.companions)):
            candidate = ROOT / name
            if not candidate.is_file() or candidate.is_symlink():
                continue
            digests[name] = _digest(candidate)
            if _is_binary(candidate):
                binary.append(name)
    orphan_references: dict[str, tuple[str, ...]] = {}
    for fixture in fixtures:
        if fixture.disposition != "delete" or Path(fixture.source).name == ".gitkeep":
            continue
        name = Path(fixture.source).name
        orphan_references[fixture.source] = tuple(
            sorted(path for path, text in sources.items() if name in text)
        )
    return Audit(
        tracked_data=tuple(tracked_data),
        tracked_all=tuple(tracked_all),
        ignored_data=tuple(ignored_data),
        untracked_data=tuple(untracked_data),
        root_reads=root_reads,
        consumers=consumers,
        orphan_references=orphan_references,
        digests=digests,
        binary=tuple(sorted(binary)),
    )


def _problems(policy: dict[str, Any], audit: Audit) -> list[str]:
    fixtures = _fixtures(policy)
    if not fixtures:
        return ["missing fixture ownership surface"]
    problems: list[str] = []
    sources = [fixture.source for fixture in fixtures]
    duplicates = sorted({source for source in sources if sources.count(source) > 1})
    if duplicates:
        problems.append(f"duplicate fixture owners: {duplicates}")
    declared_root = sorted(source for source in sources if source.startswith("data/"))
    actual_root = sorted(audit.tracked_data)
    if declared_root != actual_root:
        problems.append(
            f"root fixture ownership: expected {declared_root}, found {actual_root}"
        )
    for fixture in fixtures:
        if not all((fixture.source, fixture.target, fixture.owner, fixture.item)):
            problems.append(f"incomplete fixture owner: {fixture.source!r}")
        if fixture.source not in audit.tracked_all:
            problems.append(f"missing fixture source: {fixture.source}")
        if fixture.target.startswith("config/"):
            problems.append(
                f"fixture targets product config: {fixture.source} -> {fixture.target}"
            )
        for name, declared in (
            (fixture.source, fixture.sha256),
            *((c.path, c.sha256) for c in fixture.companions),
        ):
            # Bytes nobody can read in a diff are reviewed by their digest or
            # not at all. An in-place edit of a binary fixture is how a schema
            # change once shipped inside an opaque blob.
            if name in audit.binary and not declared:
                problems.append(f"binary fixture without a recorded sha256: {name}")
            if not declared:
                continue
            actual_digest = audit.digests.get(name)
            if actual_digest is None:
                problems.append(f"missing fixture file: {name}")
            elif actual_digest != declared:
                problems.append(
                    f"fixture bytes changed without review: {name}: "
                    f"recorded {declared}, found {actual_digest}"
                )
        for companion in fixture.companions:
            if companion.path not in audit.tracked_all:
                problems.append(f"untracked fixture companion: {companion.path}")
        if fixture.disposition == "retain":
            actual = audit.consumers.get(fixture.source, ())
            if not actual:
                problems.append(
                    f"retained fixture has no executable consumer: {fixture.source}"
                )
            if actual != fixture.symbols:
                problems.append(
                    f"stale fixture callers: {fixture.source}: "
                    f"expected {fixture.symbols}, found {actual}"
                )
        elif fixture.disposition == "delete":
            references = audit.orphan_references.get(fixture.source, ())
            if references:
                problems.append(
                    f"orphan fixture still has callers: {fixture.source}: {references}"
                )
        else:
            problems.append(
                f"invalid fixture disposition: {fixture.source}: {fixture.disposition!r}"
            )
    expected_reads = tuple(policy.get("root_reads", []))
    if audit.root_reads != expected_reads:
        problems.append(
            f"root data reads: expected {expected_reads}, found {audit.root_reads}"
        )
    if audit.ignored_data or audit.untracked_data:
        problems.append(
            f"unowned data files: ignored={audit.ignored_data}, untracked={audit.untracked_data}"
        )
    return problems


def _synthetic(**changes: object) -> Audit:
    values: dict[str, object] = {
        "tracked_data": ("data/fixtures/live.html",),
        "tracked_all": ("data/fixtures/live.html",),
        "ignored_data": (),
        "untracked_data": (),
        "root_reads": (),
        "consumers": {"data/fixtures/live.html": ("test_live",)},
        "orphan_references": {},
        "digests": {"data/fixtures/live.html": "a" * 64},
        "binary": (),
    }
    values.update(changes)
    return Audit(**values)  # type: ignore[arg-type]


def _synthetic_policy(**changes: object) -> dict[str, Any]:
    fixture = {
        "source": "data/fixtures/live.html",
        "target": "tests/fixtures/live.html",
        "disposition": "retain",
        "owner": "test owner",
        "item": "S05-004",
        "consumer": "tests/test_live.py",
        "symbols": ["test_live"],
    }
    fixture.update(changes)
    return {"version": 1, "root_reads": [], "fixture": [fixture]}


@pytest.mark.parametrize(
    ("policy", "audit", "message"),
    [
        ({"version": 1, "root_reads": []}, _synthetic(), "missing fixture ownership"),
        (
            _synthetic_policy(),
            _synthetic(
                tracked_data=("data/fixtures/live.html", "data/fixtures/rogue.json")
            ),
            "root fixture ownership",
        ),
        (
            _synthetic_policy(target="config/fixtures/live.html"),
            _synthetic(),
            "product config",
        ),
        (
            _synthetic_policy(),
            _synthetic(consumers={"data/fixtures/live.html": ()}),
            "no executable consumer",
        ),
        (
            _synthetic_policy(symbols=["old_test"]),
            _synthetic(),
            "stale fixture callers",
        ),
        (
            _synthetic_policy(disposition="delete", consumer="", symbols=[]),
            _synthetic(
                consumers={},
                orphan_references={"data/fixtures/live.html": ("tests/test_rogue.py",)},
            ),
            "orphan fixture still has callers",
        ),
        (
            {
                **_synthetic_policy(),
                "root_reads": ["tests/test_live.py:data/fixtures/live.html"],
            },
            _synthetic(),
            "root data reads",
        ),
        (
            _synthetic_policy(sha256="a" * 64),
            _synthetic(digests={"data/fixtures/live.html": "b" * 64}),
            "fixture bytes changed without review",
        ),
        (
            _synthetic_policy(),
            _synthetic(binary=("data/fixtures/live.html",)),
            "binary fixture without a recorded sha256",
        ),
        (
            _synthetic_policy(
                companion=[{"path": "data/fixtures/live.bin", "sha256": "c" * 64}]
            ),
            _synthetic(),
            "untracked fixture companion",
        ),
        (
            _synthetic_policy(),
            _synthetic(untracked_data=("data/fixtures/rogue.json",)),
            "unowned data files",
        ),
    ],
)
def test_each_prohibited_fixture_shape_is_observed_red(
    policy: dict[str, Any], audit: Audit, message: str
) -> None:
    assert any(message in problem for problem in _problems(policy, audit)), RATIONALE


def test_current_fixture_ownership_is_exact() -> None:
    policy = tomllib.loads(POLICY.read_text(encoding="utf-8"))
    assert policy.get("version") == 1
    fixtures = _fixtures(policy)
    problems = _problems(policy, _audit(fixtures))
    assert not problems, RATIONALE + "\n" + "\n".join(problems)
