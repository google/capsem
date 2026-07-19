"""Cross-agent skill index contract.

Every agent instruction file (CLAUDE.md for Claude, GEMINI.md for Gemini,
AGENTS.md for Codex) must stay consistent with the canonical `skills/`
library: complete indexes, no dangling references, intact discovery
symlinks. This guards the drift where a skill exists but is invisible to
one agent, or an agent file references a skill that no longer exists.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).parent.parent

# Files that must carry the complete skill index table.
FULL_INDEX_FILES = ("CLAUDE.md", "GEMINI.md")

# All agent instruction files: references must resolve, pointer must exist.
AGENT_FILES = ("CLAUDE.md", "GEMINI.md", "AGENTS.md")

# Per-agent discovery roots that must symlink to the canonical skills/.
DISCOVERY_ROOTS = (".claude", ".codex", ".gemini", ".agents")

# A backticked `/kebab-name` token (no internal slashes or dots) is a skill
# reference; paths like `/vm/terminal/index.html` do not match.
SKILL_REFERENCE = re.compile(r"`/([a-z0-9]+(?:-[a-z0-9]+)*)`")


def checked_in_skills() -> set[str]:
    root = PROJECT_ROOT / "skills"
    return {
        entry.name
        for entry in root.iterdir()
        if entry.is_dir() and not entry.name.startswith(".")
    }


@pytest.mark.parametrize("index_file", FULL_INDEX_FILES)
def test_every_skill_is_indexed(index_file: str) -> None:
    text = (PROJECT_ROOT / index_file).read_text(encoding="utf-8")
    missing = sorted(
        name for name in checked_in_skills() if f"`/{name}`" not in text
    )
    assert not missing, (
        f"{index_file} is missing skill index entries for: {missing}. "
        "Every directory under skills/ must appear in every agent index."
    )


@pytest.mark.parametrize("agent_file", AGENT_FILES)
def test_no_dangling_skill_references(agent_file: str) -> None:
    text = (PROJECT_ROOT / agent_file).read_text(encoding="utf-8")
    skills = checked_in_skills()
    dangling = sorted(
        name
        for name in set(SKILL_REFERENCE.findall(text))
        if name not in skills
    )
    assert not dangling, (
        f"{agent_file} references skills that do not exist under skills/: "
        f"{dangling}"
    )


@pytest.mark.parametrize("root", DISCOVERY_ROOTS)
def test_discovery_symlink_points_at_canonical_skills(root: str) -> None:
    link = PROJECT_ROOT / root / "skills"
    assert link.is_symlink(), f"{root}/skills must be a symlink to ../skills"
    assert link.resolve() == (PROJECT_ROOT / "skills").resolve(), (
        f"{root}/skills must resolve to the canonical skills/ directory"
    )


@pytest.mark.parametrize("index_file", FULL_INDEX_FILES)
def test_index_files_carry_the_common_contract_pointer(index_file: str) -> None:
    text = (PROJECT_ROOT / index_file).read_text(encoding="utf-8")
    assert "AGENTS.md" in text, f"{index_file} must point at AGENTS.md"
    for marker in ("release-qualification.yaml", "capsem-logger"):
        assert marker in text, (
            f"{index_file} must summarize the {marker} hard contract in its "
            "AGENTS.md pointer so every agent sees it without a second read"
        )
