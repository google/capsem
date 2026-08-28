"""Reject obsolete repository path literals across tracked text surfaces."""

from __future__ import annotations

import hashlib
import re
import subprocess
import tomllib
from collections.abc import Mapping
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
DEBT = Path(__file__).with_name("repository_path_debt.toml")

PATH_OWNERSHIP_RATIONALE = """\
Moved repository paths are API-like ownership boundaries. A stale literal in
Rust, Python, TOML, YAML, shell, Docker/Jinja, JavaScript, Markdown, a skill,
or package data can make one build lane read a different tree from another.
Temporary migration debt is an exact, content-hashed, symmetrically reconciled
inventory: new debt, changed debt, and stale ledger entries all fail.
"""

OBSOLETE_PREFIXES = (
    "assets/",
    "bench/",
    "data/fixtures/",
    "docker/",
    "docs/",
    "frontend/",
    "graphics/",
    "packages/",
    "release-site/",
    "scripts/",
    "security/keys/",
    "site/",
    "sprints/",
    "src/capsem/",
    "test-artifacts/",
    "tmp/",
)
OBSOLETE_ROOT_FILES = ("entitlements.plist", "pyproject.toml", "uv.lock")
EXCLUDED_POLICY_FILES = frozenset(
    {
        "tests/citadel/repository_path_debt.toml",
        "tests/citadel/repository_surface_ownership.toml",
        "tests/citadel/test_release_authority_is_canonical.py",
        "tests/citadel/test_repository_path_ownership.py",
        "tests/citadel/test_repository_surface_ownership.py",
    }
)
PATH_LITERAL = re.compile(
    rf"(?<![A-Za-z0-9_./-])(?:\.\./|\./)*(?P<path>"
    rf"(?:{'|'.join(re.escape(prefix) for prefix in OBSOLETE_PREFIXES)})"
    rf"[A-Za-z0-9_.@+/-]*)"
)
ROOT_FILE_LITERAL = re.compile(
    rf"(?<![A-Za-z0-9_./-])(?:\.\./|\./)*(?P<path>"
    rf"{'|'.join(re.escape(path) for path in OBSOLETE_ROOT_FILES)})"
    rf"(?![A-Za-z0-9_.-])"
)


def _tokens(text: str) -> list[str]:
    matches = [match.group("path") for match in PATH_LITERAL.finditer(text)]
    matches.extend(match.group("path") for match in ROOT_FILE_LITERAL.finditer(text))
    return sorted(matches)


def _digest(tokens: list[str]) -> str:
    canonical = "\0".join(sorted(tokens)).encode()
    return hashlib.sha256(canonical).hexdigest()


def _debt_by_file(sources: Mapping[str, str]) -> dict[str, str]:
    found: dict[str, str] = {}
    for path, text in sources.items():
        tokens = _tokens(text)
        if tokens:
            found[path] = _digest(tokens)
    return found


def _tracked_text_sources(root: Path = ROOT) -> dict[str, str]:
    output = subprocess.run(
        ("git", "ls-files", "-s", "-z"),
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout
    sources: dict[str, str] = {}
    for record in output.split(b"\0"):
        if not record:
            continue
        metadata, raw_path = record.split(b"\t", 1)
        mode = metadata.split(b" ", 1)[0]
        path = raw_path.decode("utf-8")
        if mode == b"120000" or path in EXCLUDED_POLICY_FILES:
            continue
        raw = (root / path).read_bytes()
        if b"\0" in raw:
            continue
        try:
            sources[path] = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
    assert sources, PATH_OWNERSHIP_RATIONALE + "\nempty tracked text surface"
    return sources


def _reconcile(found: Mapping[str, str], expected: Mapping[str, str]) -> list[str]:
    found_paths = set(found)
    expected_paths = set(expected)
    problems = [f"add debt entry: {path}" for path in sorted(found_paths - expected_paths)]
    problems.extend(
        f"remove stale debt entry: {path}" for path in sorted(expected_paths - found_paths)
    )
    problems.extend(
        f"update changed debt entry: {path}"
        for path in sorted(found_paths & expected_paths)
        if found[path] != expected[path]
    )
    return problems


@pytest.mark.parametrize(
    ("surface", "source", "expected"),
    [
        ("python", 'Path("scripts/check.py")', "scripts/check.py"),
        ("rust-parent-relative", 'include_str!("../../security/keys/capsem-ca.key")', "security/keys/capsem-ca.key"),
        ("toml", 'source = "src/capsem/gate/cli.py"', "src/capsem/gate/cli.py"),
        ("yaml", "path: frontend/src/lib/api.ts", "frontend/src/lib/api.ts"),
        ("markdown", "See [the plan](docs/architecture.md).", "docs/architecture.md"),
        ("docker-jinja", "COPY scripts/{{ helper }} /usr/bin/helper", "scripts/"),
    ],
)
def test_obsolete_literals_are_found_across_languages(
    surface: str, source: str, expected: str
) -> None:
    assert expected in _tokens(source), PATH_OWNERSHIP_RATIONALE + f"\n{surface}"


def test_operating_system_tmp_path_is_not_repository_debt() -> None:
    assert _tokens("write the socket to /tmp/capsem.sock") == [], (
        PATH_OWNERSHIP_RATIONALE
    )


def test_external_url_path_is_not_repository_debt() -> None:
    assert _tokens("https://example.test/docs/reference/index.html") == [], (
        PATH_OWNERSHIP_RATIONALE
    )


@pytest.mark.parametrize(
    ("found", "expected", "message"),
    [
        ({"new.py": "1"}, {}, "add debt entry: new.py"),
        ({}, {"gone.py": "1"}, "remove stale debt entry: gone.py"),
        ({"changed.py": "2"}, {"changed.py": "1"}, "update changed debt entry: changed.py"),
    ],
)
def test_debt_reconciliation_is_symmetric(
    found: dict[str, str], expected: dict[str, str], message: str
) -> None:
    assert _reconcile(found, expected) == [message], PATH_OWNERSHIP_RATIONALE


def test_empty_source_surface_fails_closed() -> None:
    with pytest.raises(AssertionError, match="empty tracked text surface"):
        _scan_sources([])


def _scan_sources(paths: list[Path]) -> dict[str, str]:
    assert paths, PATH_OWNERSHIP_RATIONALE + "\nempty tracked text surface"
    sources = {str(path): path.read_text(encoding="utf-8") for path in paths}
    return _debt_by_file(sources)


def test_current_obsolete_path_debt_is_exact_and_reasoned() -> None:
    policy = tomllib.loads(DEBT.read_text(encoding="utf-8"))
    prefixes = policy["prefixes"]
    expected_prefixes = set(OBSOLETE_PREFIXES) | set(OBSOLETE_ROOT_FILES)
    assert set(prefixes) == expected_prefixes, (
        PATH_OWNERSHIP_RATIONALE + "\nprefix reason inventory is not symmetric"
    )
    for prefix, decision in prefixes.items():
        assert decision["item"].startswith("S"), (
            PATH_OWNERSHIP_RATIONALE + f"\n{prefix}: missing Sprinty owner"
        )
        assert len(decision["reason"]) >= 24, (
            PATH_OWNERSHIP_RATIONALE + f"\n{prefix}: reason is not reviewable"
        )

    found = _debt_by_file(_tracked_text_sources())
    expected = policy["files"]
    problems = _reconcile(found, expected)
    assert not problems, PATH_OWNERSHIP_RATIONALE + "\n" + "\n".join(problems)
