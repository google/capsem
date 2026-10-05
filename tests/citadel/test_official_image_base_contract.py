"""A required immutable parent must not become a mutable default to silence lint."""

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
DIRECTIVE = "# check=skip=InvalidDefaultArgInFrom"
BASE_RATIONALE = (
    "Official image builds supply BASE explicitly. BuildKit checks its absent "
    "default and warned during reference-image qualification. Use the same "
    "named-check directive as the runtime templates; preserve required BASE "
    "rather than introducing an unpinned fallback."
)
DERIVED = tuple(
    path for path in sorted((ROOT / "images").glob("*/Dockerfile"))
    if re.search(r"^ARG BASE(?:\s|=|$)", path.read_text(), re.MULTILINE)
)


def _valid(source: str) -> bool:
    return (
        source.startswith(DIRECTIVE + "\n")
        and re.search(r"^ARG BASE$", source, re.MULTILINE) is not None
        and re.search(r"^FROM \$\{BASE\}$", source, re.MULTILINE) is not None
    )


@pytest.mark.parametrize("path", DERIVED, ids=lambda path: path.parent.name)
def test_official_images_keep_their_required_parent_warning_free(path: Path) -> None:
    assert _valid(path.read_text()), f"{path.relative_to(ROOT)}: {BASE_RATIONALE}"


def test_parent_guard_rejects_neutralized_inputs() -> None:
    assert DERIVED, BASE_RATIONALE
    source = DERIVED[0].read_text()
    assert _valid(source), BASE_RATIONALE
    for changed in (
        "",
        source.replace(DIRECTIVE, "# a comment hides the directive", 1),
        source.replace("ARG BASE\n", "ARG BASE=debian:latest\n", 1),
        source.replace("FROM ${BASE}", "FROM debian:latest", 1),
    ):
        assert not _valid(changed), BASE_RATIONALE
