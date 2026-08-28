"""Citadel guard: a skill description is routing text, not documentation.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. Why this one exists is in `SKILL_DESCRIPTION_RATIONALE` below, stated
there so a violation prints it rather than a bare character count.
"""

from __future__ import annotations

import re
from pathlib import Path

from capsem_builder.gate import config as gate_config

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SETTINGS = gate_config.load(PROJECT_ROOT).audits

DESCRIPTION = re.compile(r"^description:\s*(.+?)(?=\n[a-z_-]+:|\n---)", re.S | re.M)

SKILL_DESCRIPTION_RATIONALE = """\
Skill descriptions are loaded into every session; keep them one sentence of
what, one of when.

Two costs, and the smaller one is the tokens. A description is the text a
router picks from, so length is not neutral: thirty-four paragraphs
discriminate worse than thirty-four sentences, and choosing the wrong skill
costs far more than carrying a few hundred extra characters.

The set was uniformly over rather than a few offenders -- median 320
characters, every one of the 34 above 150 -- because each ended with a
"Covers X, Y, Z" enumeration restating its own body. Trimming that one habit
halved the total.

What belongs here:

    what it is        one clause
    when to use it    the trigger, in the words someone would actually use
    where to go       only when two skills genuinely collide, e.g. dev-start
                      pointing at dev-setup, or dev-bug-review at dev-debugging

What does not: any list of what the skill covers. The body already documents
that, and nothing routes on it.

Bodies are governed separately and loosely. They were already healthy, and the
split-to-references/ pattern handles the long ones.

See skills/meta-skill-creation/SKILL.md and config/gate.toml [audits].
"""


def _skills() -> list[tuple[str, str, int]]:
    """Every skill as (name, description, body line count)."""
    found = []
    for path in sorted((PROJECT_ROOT / SETTINGS.skills_dir).glob("*/SKILL.md")):
        text = path.read_text()
        match = DESCRIPTION.search(text)
        description = " ".join(match.group(1).split()) if match else ""
        found.append((path.parent.name, description, len(text.splitlines())))
    return found


def test_there_are_skills_to_measure() -> None:
    """A budget guard over an empty directory asserts nothing."""
    assert _skills(), "no skills found; this contract would be vacuous"


def test_every_skill_description_fits_the_session_budget() -> None:
    ceiling = SETTINGS.max_skill_description_chars
    oversized = [
        f"{name}: {size} chars (ceiling {ceiling})"
        for name, description, _lines in _skills()
        if (size := len(description)) > ceiling
    ]
    assert not oversized, SKILL_DESCRIPTION_RATIONALE + "\n" + "\n".join(oversized)


def test_every_skill_has_a_description_worth_routing_on() -> None:
    """An empty or near-empty description is the other failure mode.

    A ceiling alone is satisfiable by deleting the text, which would make the
    skill unroutable rather than concise.
    """
    missing = [
        f"{name}: {len(description)} chars"
        for name, description, _lines in _skills()
        if len(description) < 40
    ]
    assert not missing, (
        SKILL_DESCRIPTION_RATIONALE + "\nnot enough to route on:\n" + "\n".join(missing)
    )


def test_no_skill_body_grows_past_its_ceiling() -> None:
    ceiling = SETTINGS.max_skill_body_lines
    oversized = [
        f"{name}: {lines} lines (ceiling {ceiling}); split detail into references/"
        for name, _description, lines in _skills()
        if lines > ceiling
    ]
    assert not oversized, SKILL_DESCRIPTION_RATIONALE + "\n" + "\n".join(oversized)
