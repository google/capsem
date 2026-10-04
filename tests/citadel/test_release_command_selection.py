"""Citadel guard for choosing release dispatchers instead of local install.

The local install recipe sits near the release commands in the agent quick
start. Without an explicit executable warning, an agent can mistake hands-on
testing for a release prerequisite. The prerequisite is `just test <commit>`,
whose glow-up already installs the exact packages.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]

INSTALL_WARNING = (
    "Agent: optional hands-on local testing only; 'just install' does not "
    "qualify or unblock a release. Releases need 'just test <commit>' to pass "
    "first, then 'just release-binaries ...' or 'just release-assets ...'."
)

RATIONALE = """\
Local install was presented as a release prerequisite.

`just install` mutates one developer Mac for optional hands-on testing. The
release prerequisite is a passing `just test <commit>`, whose glow-up installs
the exact packages; hosted release lanes then build, install, qualify, and
publish. An agent that inserts local install before a release delays dispatch
and adds machine-specific state without adding release authority.
"""


def _recipe_body(name: str) -> list[str]:
    lines = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8").splitlines()
    start = next(index for index, line in enumerate(lines) if line == f"{name}:")
    body = []
    for line in lines[start + 1 :]:
        if line and not line[0].isspace():
            break
        if line.strip():
            body.append(line.strip())
    return body


def _normalized(path: str) -> str:
    text = (PROJECT_ROOT / path).read_text(encoding="utf-8")
    return re.sub(r"\s+", " ", text)


def test_install_warns_that_release_dispatch_is_direct() -> None:
    assert _recipe_body("install") == [
        f'@echo "{INSTALL_WARNING}"',
        "uv run --project build_system --frozen capsem-gate local-install",
    ], RATIONALE


def test_agent_contracts_forbid_install_as_release_prerequisite() -> None:
    for path in ("AGENTS.md", "skills/dev-just/SKILL.md"):
        contract = _normalized(path)
        assert "just install" in contract, RATIONALE
        assert "hands-on" in contract, RATIONALE
        assert "just release-binaries" in contract, RATIONALE
        assert "just release-assets" in contract, RATIONALE
