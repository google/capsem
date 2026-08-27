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

from pathlib import Path

from capsem.gate.shelllex import heredocs
from capsem.gate.shellsurfaces import workflow_bodies

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"


def test_no_workflow_heredoc_substitutes_a_command() -> None:
    offenders = []
    for origin, body in workflow_bodies(WORKFLOWS).items():
        for document in heredocs(body, origin=origin):
            if document.quoted:
                continue
            for number, line in document.body:
                if "`" in line.replace("\\`", "") or "$(" in line:
                    offenders.append(f"{origin}:{number}: {line.strip()}")

    assert not offenders, (
        "an unquoted heredoc runs its backticks and $(...) as commands, so the "
        "text they were meant to produce is silently replaced by nothing:\n  "
        + "\n  ".join(offenders)
        + "\nEscape them (\\`), quote the heredoc tag when nothing needs "
        "expanding, or render the document with a program a test can call."
    )
