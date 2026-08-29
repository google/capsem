"""Citadel guard: one rule about profile files, not one per language.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is the oldest shape in the book -- one fact written twice --
and it is here because it went red on trunk sixteen consecutive times before
anybody looked.
"""

from __future__ import annotations

import ast
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

RUST = ROOT / "crates/capsem-core/src/net/policy_config/profile_contract.rs"
PYTHON = ROOT / "build_system/builder/image/tools/build/stage_profile_assets.py"

#: `if self.a.is_some() != self.b.is_some()` -- the Rust contract's way of
#: saying two profile file entries must appear together or not at all.
RUST_PAIR = re.compile(r"self\.(?P<left>\w+)\.is_some\(\)\s*!=\s*self\.(?P<right>\w+)\.is_some\(\)")

#: The staging script names its pairs as data, so read the data rather than
#: the control flow around it. Parsed with `ast` for the same reason shell is
#: parsed rather than matched: a tuple of string literals is a structure, and
#: reading it as text is guessing.
PYTHON_PAIRS = "PAIRED_FILES"

PAIRS_RATIONALE = """\
`profile.files` entries that must appear together are declared twice: once in
the Rust profile contract, once in the Python staging check. Two spellings of
one fact, in two languages, with nothing comparing them.

They had already drifted when this guard was written. Rust paired both
`python_requirements`/`python_requirements_lock` and
`npm_packages`/`npm_package_lock`; staging checked only the first. A profile
carrying npm packages without their lock would pass staging and be refused
later by the contract, or not refused at all, depending on which side saw it
first.

The cost of the half that *was* enforced is on the record. Adding the Python
check made `test-install` fail on every push to trunk -- sixteen consecutive
runs -- because the rule was correct and the already-published stable profile
predated it. Staging now identifies that legacy cohort by the absence of its
source commit; newly authored profiles still fail closed, so compatibility
cannot become a permanent escape hatch.

This does not merge the two implementations; a Rust type and a staging script
legitimately live apart. It requires them to agree about *which* pairs exist,
which is the only part that can silently diverge.

See crates/capsem-core/src/net/policy_config/profile_contract.rs and the
image-owned stage_profile_assets.py module.
"""


def rust_pairs() -> set[frozenset[str]]:
    found = {
        frozenset({match.group("left"), match.group("right")})
        for match in RUST_PAIR.finditer(RUST.read_text(encoding="utf-8"))
    }
    assert found, f"no pairing rules found in {RUST.name}; the guard has gone blind"
    return found


def python_pairs() -> set[frozenset[str]]:
    tree = ast.parse(PYTHON.read_text(encoding="utf-8"))
    for node in ast.walk(tree):
        if not isinstance(node, ast.Assign):
            continue
        names = [t.id for t in node.targets if isinstance(t, ast.Name)]
        if PYTHON_PAIRS not in names or not isinstance(node.value, ast.Tuple):
            continue
        found = set()
        for element in node.value.elts:
            if not isinstance(element, ast.Tuple) or len(element.elts) != 2:
                continue
            words = [w.value for w in element.elts if isinstance(w, ast.Constant)]
            if len(words) == 2:
                found.add(frozenset(words))
        return found
    return set()


def test_both_sides_pair_the_same_profile_files() -> None:
    rust, python = rust_pairs(), python_pairs()
    missing = sorted(sorted(pair) for pair in rust - python)
    extra = sorted(sorted(pair) for pair in python - rust)
    assert not missing and not extra, (
        PAIRS_RATIONALE
        + f"\nenforced in Rust only: {missing}"
        + f"\nenforced in staging only: {extra}"
    )


def test_the_guard_can_read_both_dialects() -> None:
    """Break it here, so a refactor that blinds it fails rather than passes.

    Both extractors return an empty set against a file they cannot parse, and
    an empty set on both sides compares equal -- which is a green guard that
    checks nothing. The Rust side asserts non-empty for that reason; this
    proves each pattern matches the shape it claims to.
    """
    assert RUST_PAIR.search("if self.a.is_some() != self.b.is_some() {")
    assert not RUST_PAIR.search("if self.a.is_some() && self.b.is_some() {")

    assert python_pairs(), (
        f"{PYTHON.name} no longer declares {PYTHON_PAIRS}; both extractors "
        "returning empty compares equal, which is a guard that checks nothing"
    )


def test_the_pairs_are_actually_there() -> None:
    """A rename on either side must fail loudly rather than empty the sets."""
    rust = {frozenset(pair) for pair in rust_pairs()}
    assert frozenset({"python_requirements", "python_requirements_lock"}) in rust
    assert frozenset({"npm_packages", "npm_package_lock"}) in rust
