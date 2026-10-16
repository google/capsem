"""A value declared by config must be read, never restated in a test.

Seven gate failures in one session had one shape: a test hardcoded a number or
string that a config file authoritatively declares, the config moved, and the
test failed while reporting a broken product.

    coverage floor        asserted 65      justfile said 63
    guest kernel          demanded >= 7    build.toml pinned 6.18
    docker cache maximum fixture 30 GiB   policy maximum rose to 40
    benchmark retention   asserted (1, 6)  Cargo.toml said 0.6
    release fixtures      spelled 1.6.x    gate.toml release.line said 0.6
    service /version      startswith("1.") Cargo.toml said 0.6.0

None was a defect in Capsem. Each cost between three minutes and a full
forty-minute gate to discover, and each read as a release breaking rather than
a literal outliving the value it tracked.

This test enumerates the authorities and fails when a test restates one of
their current values. Two ways to satisfy it, both better than a literal:

  read it       tomllib.load(...)["workspace"]["package"]["version"]
  or diverge    use a fixture value that is obviously not production
                ("9.9.9"), which also proves the code under test does not
                secretly depend on the real one.
"""

from __future__ import annotations

import ast
import re
import tomllib
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parent.parent
SEARCH_ROOTS = (PROJECT_ROOT / "tests", PROJECT_ROOT / "scripts")


def _authorities() -> dict[str, tuple[str, Path]]:
    """Current value -> (what declares it, declaring file)."""
    found: dict[str, tuple[str, Path]] = {}

    cargo = PROJECT_ROOT / "Cargo.toml"
    version = tomllib.loads(cargo.read_text(encoding="utf-8"))["workspace"]["package"][
        "version"
    ]
    found[version] = ("the workspace version", cargo)

    gate_config = PROJECT_ROOT / "config" / "gate.toml"
    release = tomllib.loads(gate_config.read_text(encoding="utf-8"))["release"]
    found[release["line"]] = ("the release line", gate_config)

    build = PROJECT_ROOT / "config" / "docker" / "image" / "build.toml"
    kernel = tomllib.loads(build.read_text(encoding="utf-8"))["build"]["kernel"]
    found[kernel["version"]] = ("the exact guest kernel version", build)
    found[kernel["sha256"]] = ("the exact guest kernel source digest", build)

    return found


def _reads(text: str, declaring: Path) -> bool:
    """Whether a file reads its authority rather than restating it."""
    return declaring.name in text


def _value_strings(text: str) -> tuple[str, ...]:
    tree = ast.parse(text)
    docstrings: set[int] = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and node.body:
            first = node.body[0]
            if (
                isinstance(first, ast.Expr)
                and isinstance(first.value, ast.Constant)
                and isinstance(first.value.value, str)
            ):
                docstrings.add(id(first.value))
    return tuple(
        node.value for node in ast.walk(tree)
        if isinstance(node, ast.Constant) and isinstance(node.value, str) and id(node) not in docstrings
    )


def _restates(literals: tuple[str, ...], value: str) -> bool:
    pattern = re.compile(rf"(?<![\w.]){re.escape(value)}(?![\w.])")
    return any(pattern.search(literal) is not None for literal in literals)


@pytest.mark.parametrize("template", [
    "# historical dependency {value}\n",
    '"""Historical release {value} used another layout."""\n',
    'def history():\n    """Dependency {value} used another syntax."""\n',
    "latency_sample = {value}\n",
])
def test_the_authority_scanner_ignores_comments_docstrings_and_numeric_samples(template: str) -> None:
    line = next(value for value, (name, _) in _authorities().items() if name == "the release line")
    assert not _restates(_value_strings(template.format(value=line)), line)


@pytest.mark.parametrize("template", [
    'expected_version = "{value}"\n',
    'assert reported_version == "{value}"\n',
    'expected = \'{{"version": "{value}"}}\'\n',
])
def test_the_authority_scanner_still_rejects_behavioral_string_literals(template: str) -> None:
    line = next(value for value, (name, _) in _authorities().items() if name == "the release line")
    assert _restates(_value_strings(template.format(value=line)), line)


def test_no_test_restates_a_value_its_config_declares() -> None:
    authorities = _authorities()
    assert authorities, "no authoritative values found; this guard would pass vacuously"

    offenders: list[str] = []
    for root in SEARCH_ROOTS:
        for path in sorted(root.rglob("*.py")):
            if path.name == Path(__file__).name:
                continue
            text = path.read_text(encoding="utf-8", errors="ignore")
            literals = _value_strings(text)
            for value, (what, declaring) in authorities.items():
                if path == declaring or _reads(text, declaring):
                    continue
                # Word-bounded so 0.6 does not match inside 10.6.2.
                if _restates(literals, value):
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)} spells {value!r}, which is "
                        f"{what} declared by {declaring.relative_to(PROJECT_ROOT)}"
                    )

    assert not offenders, (
        "these restate a value their config already declares, so the config "
        "moving breaks them and reports a broken product:\n  " + "\n  ".join(offenders)
    )
