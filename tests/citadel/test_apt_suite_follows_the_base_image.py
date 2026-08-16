"""No script may name an Ubuntu suite; it reads the base image's own.

Three scripts wrote `Suites: noble ...` into apt sources. That is not a
preference, it is an override: apt then installs 24.04 packages onto whatever
base it is given. Moving the release base to Ubuntu 22.04 to lower the glibc
floor therefore changed nothing measurable -- the image reported
`PRETTY_NAME="Ubuntu 22.04.5 LTS"` while `ldd` reported `GLIBC 2.39-0ubuntu8.8`,
which is noble's libc, and every published binary still required 2.39.

A hardcoded suite cannot be spotted by reading either the Dockerfile or the
package list, because both were correct. Only the built image disagreed with
itself. So the rule is the narrow one that would have caught it: a suite is
read from `/etc/os-release`, never spelled.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SOURCES = sorted(
    [*(PROJECT_ROOT / "docker").glob("*.sh"), *(PROJECT_ROOT / "scripts").glob("*.sh")]
)

#: Every Ubuntu LTS codename this project could plausibly be pinned to.
CODENAMES = ("focal", "jammy", "noble", "questing", "resolute")

_SUITES_LINE = re.compile(r"^\s*Suites:.*$", re.MULTILINE)


def test_the_inventory_is_not_empty() -> None:
    assert len(SOURCES) >= 5
    assert any("Suites:" in path.read_text(encoding="utf-8") for path in SOURCES)


@pytest.mark.parametrize("script", SOURCES, ids=lambda path: path.name)
def test_no_script_spells_an_ubuntu_suite(script: Path) -> None:
    text = script.read_text(encoding="utf-8")
    offenders = [
        line
        for line in _SUITES_LINE.findall(text)
        if any(codename in line for codename in CODENAMES)
    ]

    assert not offenders, (
        f"{script.name} names a suite instead of reading it from the base "
        f"image: {offenders}. A spelled suite installs that release's packages "
        "onto whichever base the image actually uses, so the two can disagree "
        "silently -- which is how a 22.04 image shipped noble's glibc."
    )


@pytest.mark.parametrize("script", SOURCES, ids=lambda path: path.name)
def test_a_script_writing_apt_sources_derives_the_suite(script: Path) -> None:
    """Deriving it is the whole point; absence of a codename is not enough."""
    text = script.read_text(encoding="utf-8")
    if "Suites:" not in text:
        pytest.skip("writes no apt sources")

    assert "UBUNTU_CODENAME" in text, (
        f"{script.name} writes apt sources without reading UBUNTU_CODENAME "
        "from the base image"
    )
    assert "/etc/os-release" in text
