"""Citadel guard: every agent is held to the same contract, byte for byte.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is a mistake of *divergence*: three files that were supposed
to say the same thing, each read by a different agent, with nothing comparing
them.
"""

from __future__ import annotations

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[2]

#: The file every agent actually reads, whatever name it opens.
CANONICAL = "AGENTS.md"
#: The per-agent names, which must be links to it and never copies.
LINKED = ("CLAUDE.md", "GEMINI.md")

ONE_CONTRACT_RATIONALE = """\
The agent instruction files are one file under three names.

Claude reads CLAUDE.md, Gemini reads GEMINI.md, Codex reads AGENTS.md. When
those were three separate documents they drifted, and the drift was not a
wording difference -- their section lists were almost disjoint. Claude was
never told about the bounded-diagnostics wrapper, the serialized release
contract or the logger DB boundary. Codex was never told the code style, the
invariants, or that Rust tests live in a sibling `tests.rs`.

Nobody decided that. Three files with overlapping purpose were edited by
whoever was in front of one of them, and no reader ever saw two at once. The
result is the worst kind of rule: one that some agents are held to and others
have never heard of, where the disagreement surfaces as a review comment about
a convention the author had no way to know.

So there is one file, and the other two are symlinks. Copies are refused rather
than compared, because comparing them is what nobody ever did.

A symlink also keeps every existing reference working -- `config/gate.toml`,
`tests/test_agent_skill_index.py` and several skills name `CLAUDE.md` by path,
and `test_agent_skill_index.py` requires that path to carry the complete skill
index table.

See AGENTS.md.
"""


@pytest.mark.parametrize("name", LINKED)
def test_the_agent_file_is_a_symlink(name: str) -> None:
    """A copy is a fork with a delay on it."""
    path = PROJECT_ROOT / name
    assert path.is_symlink(), (
        ONE_CONTRACT_RATIONALE
        + f"\n{name} is a regular file. It must be a symlink to {CANONICAL}, or "
        "it will drift again."
    )


@pytest.mark.parametrize("name", LINKED)
def test_the_symlink_points_at_the_canonical_file(name: str) -> None:
    """Pointing somewhere else is the same divergence with extra steps."""
    target = (PROJECT_ROOT / name).readlink()
    assert target.name == CANONICAL and not target.is_absolute(), (
        ONE_CONTRACT_RATIONALE
        + f"\n{name} points at {target}, not a relative {CANONICAL}. An absolute "
        "target breaks in every clone but the one it was made in."
    )


@pytest.mark.parametrize("name", LINKED)
def test_every_agent_reads_the_same_bytes(name: str) -> None:
    """The property that matters, asserted directly rather than inferred.

    A symlink is the mechanism; identical content is the point. Checked
    separately so the guard still means something if the mechanism changes.
    """
    assert (PROJECT_ROOT / name).read_bytes() == (PROJECT_ROOT / CANONICAL).read_bytes(), (
        ONE_CONTRACT_RATIONALE + f"\n{name} does not resolve to the same bytes as {CANONICAL}"
    )


def test_the_canonical_file_is_the_real_one() -> None:
    """Guards against the loop where every name is a link and none is a file."""
    canonical = PROJECT_ROOT / CANONICAL
    assert canonical.is_file() and not canonical.is_symlink(), (
        ONE_CONTRACT_RATIONALE + f"\n{CANONICAL} must be the regular file the others point at"
    )


def test_git_stores_them_as_links_not_copies() -> None:
    """A symlink on disk that Git recorded as a blob is a copy again on clone.

    Git mode `120000` is a symlink; `100644` is a regular file. Checking the
    working tree alone would pass for a checkout that had already materialized
    the copies, which is precisely the state a broken commit produces.
    """
    import subprocess

    listed = subprocess.run(
        ["git", "ls-files", "-s", "--", *LINKED, CANONICAL],
        cwd=PROJECT_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    modes = {line.split()[3]: line.split()[0] for line in listed if line.strip()}

    for name in LINKED:
        assert modes.get(name) == "120000", (
            ONE_CONTRACT_RATIONALE
            + f"\ngit records {name} as mode {modes.get(name)}, not a symlink (120000); "
            "it would arrive as a divergent copy in every fresh clone"
        )
    assert modes.get(CANONICAL) == "100644", (
        ONE_CONTRACT_RATIONALE + f"\ngit records {CANONICAL} as mode {modes.get(CANONICAL)}"
    )
