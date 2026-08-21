"""A backtick inside an unquoted heredoc is a command, not markdown.

`create-release` wrote its release notes with ``cat > "$notes" <<EOF`` and a
line reading ``Qualified source: `$SOURCE_COMMIT`.`` -- meant as inline code.
An unquoted heredoc performs command substitution, so bash ran the commit hash
as a program, printed `command not found`, substituted nothing, and exited 0.
`set -euo pipefail` does not catch it. The notes would have shipped reading
"Qualified source: ." with the hash silently gone.

Quoting the tag is not always the fix: a summary heredoc that interpolates
`$COV` needs expansion. Backticks are the part that is never wanted.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"

_OPENER = re.compile(r"<<-?\s*(?P<quote>['\"]?)(?P<tag>[A-Za-z_][A-Za-z0-9_]*)(?P=quote)")


def _unquoted_heredoc_bodies(text: str) -> list[tuple[int, str]]:
    """Every line inside a heredoc whose tag was not quoted."""
    lines = text.splitlines()
    found: list[tuple[int, str]] = []
    index = 0
    while index < len(lines):
        opener = _OPENER.search(lines[index])
        if opener is None:
            index += 1
            continue
        tag, quoted = opener.group("tag"), bool(opener.group("quote"))
        index += 1
        while index < len(lines) and lines[index].strip() != tag:
            if not quoted:
                found.append((index + 1, lines[index]))
            index += 1
        index += 1
    return found


def test_no_workflow_heredoc_substitutes_a_command() -> None:
    offenders = []
    for workflow in sorted(WORKFLOWS.glob("*.y*ml")):
        text = workflow.read_text(encoding="utf-8")
        for number, line in _unquoted_heredoc_bodies(text):
            if "`" in line.replace("\\`", "") or "$(" in line:
                offenders.append(f"{workflow.name}:{number}: {line.strip()}")

    assert not offenders, (
        "an unquoted heredoc runs its backticks and $(...) as commands, so the "
        "text they were meant to produce is silently replaced by nothing:\n  "
        + "\n  ".join(offenders)
        + "\nEscape them (\\`), quote the heredoc tag when nothing needs "
        "expanding, or render the document with a program a test can call."
    )
