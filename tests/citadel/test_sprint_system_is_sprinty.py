"""Keep Sprinty as the repository's only active development-sprint system."""

from __future__ import annotations

from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SPRINT_SKILL = "skills/dev-sprint/SKILL.md"

SPRINTY_RATIONALE = """
Sprint state must survive agent sessions without becoming a second source tree.
Sprinty owns goals, dependencies, notes, evidence, coverage, and closeout; Git
owns code and permanent history. Instructions that create tracked sprints/ or
tmp/ planning trees make the two ledgers diverge and can resurrect obsolete
release authority after the repository has deliberately removed it.
""".strip()

REQUIRED_SPRINTY_CALLS = (
    "mcp__sprinty.info()",
    "mcp__sprinty.sprint_resume()",
    "mcp__sprinty.sprint_new()",
    "mcp__sprinty.subsprint_new()",
    "mcp__sprinty.item_add()",
    "mcp__sprinty.item_done()",
    "mcp__sprinty.sprint_close()",
)

FORBIDDEN_INSTRUCTIONS = (
    "mkdir -p sprints/",
    "Write `sprints/",
    "Create `sprints/",
    "sprints/<sprint-name>/",
    "The `sprints/` directory is git-tracked",
    "tmp/build_sprint",
    "tmp/" + "release-spec.md",
)


def _skill_documents() -> dict[str, str]:
    return {
        path.relative_to(ROOT).as_posix(): path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "skills").glob("*/SKILL.md"))
    }


def _problems(documents: dict[str, str]) -> list[str]:
    problems: list[str] = []
    sprint = documents.get(SPRINT_SKILL, "")
    for call in REQUIRED_SPRINTY_CALLS:
        if call not in sprint:
            problems.append(f"{SPRINT_SKILL}: missing required call {call}")

    for path, text in documents.items():
        for instruction in FORBIDDEN_INSTRUCTIONS:
            if instruction in text:
                problems.append(f"{path}: obsolete planning instruction {instruction}")
    return problems


@pytest.mark.parametrize("instruction", FORBIDDEN_INSTRUCTIONS)
def test_each_obsolete_planning_instruction_is_observed_red(instruction: str) -> None:
    valid = "\n".join(REQUIRED_SPRINTY_CALLS)
    problems = _problems({SPRINT_SKILL: f"{valid}\n{instruction}\n"})
    assert any(instruction in problem for problem in problems), SPRINTY_RATIONALE


def test_missing_sprinty_lifecycle_call_is_observed_red() -> None:
    incomplete = "\n".join(REQUIRED_SPRINTY_CALLS[:-1])
    problems = _problems({SPRINT_SKILL: incomplete})
    assert any("sprint_close" in problem for problem in problems), SPRINTY_RATIONALE


def test_repository_uses_sprinty_exclusively() -> None:
    problems = _problems(_skill_documents())
    assert not problems, SPRINTY_RATIONALE + "\n" + "\n".join(problems)
