"""A gate's result must not be read from the last line of a multi-part output.

Two shapes of the same mistake, both of which report success while the thing
being measured failed:

1. `$?` after a pipe is the *pipe's* status. `just test | tail` reports what
   `tail` did. Under `set -o pipefail` the pipeline adopts the first non-zero
   status, which is why every bash recipe here sets it.

2. `tail -n1` across a multi-part result returns the last part, not the whole.
   `cargo test -p capsem-service` runs three test binaries; the last prints
   `0 passed`, so `| tail -1` reads as though the crate had no tests at all
   while 91 and 264 passed above it.

This guards committed scripts, workflows, and recipes. It cannot guard an
agent's ad-hoc shell -- the rule for that lives in `/dev-testing` -- so it is a
regression guard rather than a detector, and it asserts it actually inspected
something so it cannot pass by finding nothing.
"""

from __future__ import annotations

import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent

# A command whose exit status is the thing being measured, piped into a reader
# that discards it.
GATE_PIPED_TO_READER = re.compile(
    r"(?:cargo\s+(?:test|clippy|build|check)|pytest|uv\s+run\s+pytest|just\s+[a-z_][a-z0-9_-]*)"
    r"[^|\n]*\|\s*(?:head|tail)\b"
)

SHELL_SOURCES = ("*.sh",)
WORKFLOW_DIR = PROJECT_ROOT / ".github" / "workflows"


def _recipe_blocks() -> dict[str, str]:
    """Every justfile recipe body, keyed by recipe name."""
    text = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    blocks = re.split(r"\n(?=[a-z_@][a-zA-Z0-9_-]*(?: [^\n]*)?:)", text)
    named = {}
    for block in blocks:
        head = block.split(":", 1)[0].split()
        if head:
            named[head[0]] = block
    return named


def test_no_recipe_reads_a_gate_result_through_head_or_tail() -> None:
    recipes = _recipe_blocks()
    assert len(recipes) > 20, "justfile parse found too few recipes to be trusted"

    offenders = {
        name: GATE_PIPED_TO_READER.search(body).group(0)
        for name, body in recipes.items()
        if GATE_PIPED_TO_READER.search(body)
    }

    assert not offenders, (
        "these recipes read a gate's result through head/tail, which reports the "
        "reader's success rather than the gate's:\n  "
        + "\n  ".join(f"{name}: {snippet}" for name, snippet in sorted(offenders.items()))
    )


def test_no_script_or_workflow_reads_a_gate_result_through_head_or_tail() -> None:
    inspected = 0
    offenders = []
    sources = list((PROJECT_ROOT / "scripts").glob("*.sh"))
    sources += sorted(WORKFLOW_DIR.glob("*.yaml"))
    for path in sources:
        inspected += 1
        for match in GATE_PIPED_TO_READER.finditer(path.read_text(encoding="utf-8")):
            offenders.append(f"{path.relative_to(PROJECT_ROOT)}: {match.group(0)}")

    assert inspected > 10, "found too few scripts and workflows to inspect"
    assert not offenders, (
        "a gate's exit status must not be discarded by a pager:\n  "
        + "\n  ".join(offenders)
    )


def test_every_piping_bash_recipe_sets_pipefail() -> None:
    """Without `pipefail` a failing command upstream of a pipe is invisible."""
    offenders = [
        name
        for name, body in _recipe_blocks().items()
        if "#!/bin/bash" in body and "|" in body and "pipefail" not in body
    ]

    assert not offenders, (
        "these bash recipes pipe without `set -o pipefail`, so a failure "
        "upstream of the pipe is reported as success: " + ", ".join(sorted(offenders))
    )
