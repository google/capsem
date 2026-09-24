"""Contracts for where Rust unit tests live.

CLAUDE.md and skills/dev-testing require every Rust module to keep its unit tests
in a sibling `tests.rs`, declared with `#[cfg(test)] mod tests;`. Inline
`#[cfg(test)] mod tests { ... }` blocks bury production code: before these guards
landed, 86 files carried them and the worst single file hid 4,070 lines of tests
under 8,855 lines of production code.

These are source-shape contracts. They read the checked-in tree only, so they run
in the fast gate that `just test`, `just smoke`, ordinary CI, and both release
lanes all share.
"""

import re
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"
FIXTURE_OWNERSHIP = ROOT / "tests/citadel/fixture_ownership.toml"

INLINE_TEST_MOD = re.compile(r"^\s*(?:pub\s+)?mod\s+tests\s*\{", re.MULTILINE)
TEST_MOD_DECL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+tests\s*;", re.MULTILINE)
# `//` comments only; a `mod tests {` inside a block comment or string literal has
# never appeared here, and the guards below fail loudly if one ever does.
LINE_COMMENT = re.compile(r"//.*$", re.MULTILINE)
IGNORED_TEST = re.compile(r"#\s*\[\s*ignore(?:\s*=|\s*\])")
IGNORED_TEST_WITH_REASON = re.compile(r'#\s*\[\s*ignore\s*=\s*"[^"]+"\s*\]')
IGNORED_DOCTEST = re.compile(r"```ignore(?:\s|$)")

IGNORE_RATIONALE = (
    "#[ignore] silently removes Rust evidence from the default test run; "
    "make correctness tests deterministic and move performance scenarios to "
    "the benchmark rail. The one exception is a fixture regenerator: a test "
    "that rewrites a checked-in fixture is a tool, not evidence, and running "
    "it by default would rewrite the bytes every other test is measured "
    "against. It earns the exemption by being declared as some fixture's "
    "`regenerator` in tests/citadel/fixture_ownership.toml -- which "
    "test_fixture_ownership.py already requires to exist -- and by saying in "
    "the attribute itself why it is ignored"
)


def _declared_regenerators() -> set[str]:
    """Every file some fixture names as the thing that rewrites it.

    Read from the ownership record rather than kept as a list here, so the
    exemption is earned by a recorded ownership entry instead of by a name
    somebody added to a guard. Point a `regenerator` field somewhere else and
    the file it used to name goes back to failing below.
    """
    record = tomllib.loads(FIXTURE_OWNERSHIP.read_text(encoding="utf-8"))
    return {
        fixture["regenerator"]
        for fixture in record.get("fixture", [])
        if fixture.get("regenerator")
    }


def _ignored_test_violation(rel: str, source: str, regenerators: set[str]) -> str | None:
    """Pure over (path, text): why this file's `#[ignore]` is not allowed.

    Pure so the adversarial cases below can hand it the offending source
    directly. A tree with no offender in it cannot show that the predicate
    still finds one.
    """
    ignored = IGNORED_TEST.findall(source)
    if not ignored:
        return None
    if rel not in regenerators:
        return f"{rel} carries #[ignore] and no fixture declares it as a regenerator"
    if len(IGNORED_TEST_WITH_REASON.findall(source)) != len(ignored):
        return f"{rel} is a declared fixture regenerator but an #[ignore] there carries no reason"
    return None


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
    regenerators = _declared_regenerators()
    ignored_tests: list[str] = []
    ignored_doctests: list[str] = []

    for path in _rust_sources() + sorted(CRATES.glob("*/tests/**/*.rs")):
        source = path.read_text(encoding="utf-8")
        violation = _ignored_test_violation(_rel(path), source, regenerators)
        if violation:
            ignored_tests.append(violation)
        if IGNORED_DOCTEST.search(source):
            ignored_doctests.append(_rel(path))

    assert ignored_tests == [], IGNORE_RATIONALE + ":\n" + "\n".join(ignored_tests)
    assert ignored_doctests == [], (
        "```ignore does not even compile the example; use a runnable doctest or "
        "```no_run when execution requires process context: "
        + ", ".join(ignored_doctests)
    )


REGENERATOR = "crates/capsem-logger/tests/roundtrip/fixture_regen.rs"
WITH_REASON = '#[ignore = "rewrites the checked-in fixture"]\nfn regenerate() {}\n'
BARE = "#[ignore]\nfn regenerate() {}\n"


def test_the_declared_regenerator_is_the_one_the_record_names() -> None:
    """A guard over a declaration nobody makes asserts nothing."""
    declared = _declared_regenerators()
    assert REGENERATOR in declared, (
        f"{REGENERATOR} is no longer declared as a fixture regenerator in "
        f"{_rel(FIXTURE_OWNERSHIP)}; this guard's exemption case is vacuous"
    )
    assert IGNORED_TEST.search(
        (ROOT / REGENERATOR).read_text(encoding="utf-8")
    ), f"{REGENERATOR} no longer carries #[ignore]; it needs no exemption"


def test_an_ignored_test_outside_a_declared_regenerator_still_fails() -> None:
    """The exemption is for the regenerator, not for #[ignore] in general."""
    for source in (WITH_REASON, BARE):
        violation = _ignored_test_violation(
            "crates/capsem-core/src/net/tests.rs", source, {REGENERATOR}
        )
        assert violation is not None, source
        assert "no fixture declares it as a regenerator" in violation, violation


def test_a_declared_regenerator_must_still_say_why_it_is_ignored() -> None:
    """A bare #[ignore] says nothing; the declaration is not a blank cheque."""
    violation = _ignored_test_violation(REGENERATOR, BARE, {REGENERATOR})
    assert violation is not None
    assert "carries no reason" in violation, violation

    mixed = WITH_REASON + BARE
    assert (
        _ignored_test_violation(REGENERATOR, mixed, {REGENERATOR}) is not None
    ), "one reasoned #[ignore] must not excuse a bare one in the same file"
    assert _ignored_test_violation(REGENERATOR, WITH_REASON, {REGENERATOR}) is None


def test_moving_the_declaration_makes_the_regenerator_fail_again() -> None:
    """The mutation: point `regenerator` elsewhere and the exemption is gone.

    This is what makes the exemption earned rather than granted. The set the
    real guard uses comes from the ownership record, so an entry that moves, or
    is deleted, takes its exemption with it.
    """
    moved = {"crates/capsem-logger/tests/roundtrip/reader_queries.rs"}
    violation = _ignored_test_violation(REGENERATOR, WITH_REASON, moved)
    assert violation is not None
    assert "no fixture declares it as a regenerator" in violation, violation

    assert _ignored_test_violation(REGENERATOR, WITH_REASON, set()) is not None


def test_a_file_with_no_ignore_is_never_a_violation() -> None:
    assert _ignored_test_violation(REGENERATOR, "fn run() {}\n", set()) is None
