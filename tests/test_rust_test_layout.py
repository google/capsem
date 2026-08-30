"""Contracts for where Rust unit tests live.

CLAUDE.md and skills/dev-testing require every Rust module to keep its unit tests
in a sibling `tests.rs`, declared with `#[cfg(test)] mod tests;`. Inline
`#[cfg(test)] mod tests { ... }` blocks bury production code: before these guards
landed, 86 files carried them and the worst single file hid 4,070 lines of tests
under 8,855 lines of production code.

These are source-shape contracts. They read the checked-in tree only, so they run
in the fast gate that `just test-full`, `just smoke`, ordinary CI, and both release
lanes all share.
"""

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"

INLINE_TEST_MOD = re.compile(r"^\s*(?:pub\s+)?mod\s+tests\s*\{", re.MULTILINE)
TEST_MOD_DECL = re.compile(r"^\s*(?:pub\s+)?mod\s+tests\s*;", re.MULTILINE)
# `//` comments only; a `mod tests {` inside a block comment or string literal has
# never appeared here, and the guards below fail loudly if one ever does.
LINE_COMMENT = re.compile(r"//.*$", re.MULTILINE)
IGNORED_TEST = re.compile(r"#\s*\[\s*ignore(?:\s*=|\s*\])")
IGNORED_DOCTEST = re.compile(r"```ignore(?:\s|$)")


def _rust_sources() -> list[Path]:
    return sorted(p for p in CRATES.glob("*/src/**/*.rs") if p.is_file())


def _code(path: Path) -> str:
    return LINE_COMMENT.sub("", path.read_text(encoding="utf-8"))


def _rel(path: Path) -> str:
    return path.relative_to(ROOT).as_posix()


def test_no_production_file_carries_an_inline_test_module() -> None:
    """Unit tests belong in a sibling tests.rs, never in an inline block."""
    offenders = [
        _rel(path)
        for path in _rust_sources()
        if path.name != "tests.rs" and INLINE_TEST_MOD.search(_code(path))
    ]

    assert offenders == [], (
        "inline `mod tests { ... }` blocks bury production code; move each block "
        "to a sibling tests.rs and leave `#[cfg(test)] mod tests;` behind "
        "(see CLAUDE.md and skills/dev-testing): " + ", ".join(offenders)
    )


def test_every_tests_file_is_reachable_from_its_parent_module() -> None:
    """An undeclared tests.rs never compiles, so its tests silently stop running."""
    orphans: list[str] = []
    for tests_rs in sorted(CRATES.glob("*/src/**/tests.rs")):
        directory = tests_rs.parent
        # `mod tests;` in foo/mod.rs, a crate root, or the sibling foo.rs one level up.
        candidates = [
            directory / "mod.rs",
            directory / "main.rs",
            directory / "lib.rs",
            directory.parent / f"{directory.name}.rs",
        ]
        declared = any(
            parent.exists() and TEST_MOD_DECL.search(_code(parent))
            for parent in candidates
        )
        if not declared:
            orphans.append(_rel(tests_rs))

    assert orphans == [], (
        "these tests.rs files are not declared by any parent module, so nothing "
        "compiles or runs them: " + ", ".join(orphans)
    )


def test_no_rust_source_file_is_git_ignored() -> None:
    """An ignored source file compiles locally and is missing for everyone else.

    Splitting tests into sibling files creates new directories, and a loose
    .gitignore pattern can swallow one silently: `*_Store`, meant for .DS_Store,
    matched crates/capsem-process/src/job_store/ because macOS sets
    core.ignorecase=true. The tree still built here and would have failed on a
    fresh clone with an unresolved `mod tests;`.
    """
    sources = [_rel(p) for p in _rust_sources()]
    result = subprocess.run(
        ["git", "check-ignore", "--stdin"],
        input="\n".join(sources),
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    ignored = sorted(line for line in result.stdout.splitlines() if line.strip())

    assert ignored == [], (
        "these Rust sources are matched by .gitignore, so they would never be "
        "committed: " + ", ".join(ignored)
    )


def test_every_crate_ships_unit_tests() -> None:
    """A crate with no #[cfg(test)] anywhere is an untested surface, not a style nit."""
    untested: list[str] = []
    for manifest in sorted(CRATES.glob("*/Cargo.toml")):
        crate = manifest.parent
        has_unit = any(
            "#[cfg(test)]" in path.read_text(encoding="utf-8")
            for path in crate.glob("src/**/*.rs")
        )
        has_integration = any(crate.glob("tests/*.rs"))
        if not (has_unit or has_integration):
            untested.append(crate.name)

    assert untested == [], (
        "these crates carry no Rust tests at all; add unit tests in a sibling "
        "tests.rs or an integration test under tests/: " + ", ".join(untested)
    )


def test_rust_correctness_evidence_is_never_silently_ignored() -> None:
    """Correctness examples and tests must run on their owning test rail."""
    ignored_tests: list[str] = []
    ignored_doctests: list[str] = []

    for path in _rust_sources() + sorted(CRATES.glob("*/tests/**/*.rs")):
        source = path.read_text(encoding="utf-8")
        if IGNORED_TEST.search(source):
            ignored_tests.append(_rel(path))
        if IGNORED_DOCTEST.search(source):
            ignored_doctests.append(_rel(path))

    assert ignored_tests == [], (
        "#[ignore] silently removes Rust evidence from the default test run; "
        "make correctness tests deterministic and move performance scenarios "
        "to the benchmark rail: " + ", ".join(ignored_tests)
    )
    assert ignored_doctests == [], (
        "```ignore does not even compile the example; use a runnable doctest or "
        "```no_run when execution requires process context: "
        + ", ".join(ignored_doctests)
    )
