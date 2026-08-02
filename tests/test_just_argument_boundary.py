"""A public recipe argument may never become host shell syntax.

`just` interpolates `{{value}}` into the recipe body as *source*, and the host
shell then parses it. Every validation Python performs happens after that, so
Python cannot contain this: by the time the gate sees an argument the shell has
already run whatever was in it.

    $ just --dry-run release-binaries 'nightly; echo HOST_PWNED'
    uv run capsem-gate release-binaries nightly; echo HOST_PWNED

`exec` already demonstrates the fix -- `{{quote(CMD)}}` -- which is why it is
the one parameterized recipe this test found clean when it was written.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: Every public recipe that takes an argument, and a payload for each.
#: The payload is what an operator could paste in from a ticket or a URL.
PARAMETERIZED = [
    ("release-binaries", ["nightly; echo HOST_PWNED"]),
    ("release-profile", ["nightly; echo HOST_PWNED", "code| echo HOST_PWNED"]),
    ("logs", ["failure; echo HOST_PWNED"]),
    ("dev", ["ui; echo HOST_PWNED"]),
    ("exec", ["echo hi; echo HOST_PWNED"]),
]

#: Metacharacters that must survive as data rather than becoming syntax.
PAYLOADS = [
    "a; echo HOST_PWNED",
    "a | echo HOST_PWNED",
    "a > /tmp/capsem-boundary-probe",
    "a $(echo HOST_PWNED)",
    "a `echo HOST_PWNED`",
    "a && echo HOST_PWNED",
]


def _rendered(recipe: str, arguments: list[str]) -> str:
    result = subprocess.run(
        ["just", "--dry-run", recipe, *arguments],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
        timeout=60,
    )
    # `--dry-run` prints the recipe body it *would* run, to stderr.
    return result.stderr + result.stdout


@pytest.mark.skipif(shutil.which("just") is None, reason="just is not installed")
@pytest.mark.parametrize(("recipe", "arguments"), PARAMETERIZED)
def test_a_public_recipe_argument_cannot_become_host_shell_syntax(
    recipe: str, arguments: list[str]
) -> None:
    rendered = _rendered(recipe, arguments)

    for line in rendered.splitlines():
        stripped = line.strip()
        if "HOST_PWNED" not in stripped:
            continue
        assert "'" in stripped or '"' in stripped, (
            f"`just {recipe}` renders the payload as bare shell source:\n"
            f"  {stripped}\n"
            "Interpolate it through `{{quote(...)}}` so it stays one argument."
        )


@pytest.mark.skipif(shutil.which("just") is None, reason="just is not installed")
@pytest.mark.parametrize("payload", PAYLOADS)
def test_every_metacharacter_stays_inside_one_argument(payload: str) -> None:
    """The release surface specifically, against each metacharacter class."""
    rendered = _rendered("release-binaries", [payload])

    for line in rendered.splitlines():
        if "capsem-gate release-binaries" not in line:
            continue
        after = line.split("release-binaries", 1)[1].strip()
        assert after.startswith(("'", '"')), (
            f"channel argument is unquoted shell source: {line.strip()}"
        )


@pytest.mark.skipif(shutil.which("just") is None, reason="just is not installed")
def test_a_recipe_name_is_never_built_from_an_argument() -> None:
    """`just _dev-{{surface}}` makes the *recipe* attacker-chosen.

    Quoting cannot fix this one: the value is not an argument, it is part of
    the name being dispatched. The set of development surfaces is finite and
    known, so it is selected rather than constructed.
    """
    body = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")

    assert "just _dev-{{surface}}" not in body, (
        "the dev surface is interpolated into a recipe name; dispatch the "
        "known set instead of constructing the name from input"
    )
