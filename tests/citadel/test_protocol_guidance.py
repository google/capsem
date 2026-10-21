"""Keep the protocol and thin-shell instructions tied to their Rust owners."""

from __future__ import annotations

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
STREAM_GUIDES = (
    "skills/site-architecture/SKILL.md",
    "skills/site-architecture/references/key-files.md",
    "skills/frontend-design/SKILL.md",
    "web/docs/src/content/docs/architecture/service-architecture.md",
)
GUIDANCE_RATIONALE = (
    "Stale architecture instructions caused terminal clients to grow separate wire "
    "contracts. Protocol names come from capsem-api; desktop commands come from "
    "capsem-app's registered handler, not a second prose inventory."
)


def stream_names(text: str) -> set[str]:
    return set(re.findall(r"capsem\.stream\.v\d+", text))


def registered_commands(source: str) -> set[str]:
    handlers = re.findall(r"tauri::generate_handler!\[([^]]+)\]", source)
    assert len(handlers) == 1, GUIDANCE_RATIONALE
    return set(re.findall(r"\b[a-z_]+\b", handlers[0]))


def documented_commands(text: str) -> set[str]:
    inventories = re.findall(r"only IPC commands are ([^.]+)\.", text)
    assert len(inventories) == 1, GUIDANCE_RATIONALE
    return set(re.findall(r"`([a-z_]+)`", inventories[0]))


def test_stream_guidance_matches_the_api_declaration() -> None:
    source = (ROOT / "crates/capsem-api/src/stream.rs").read_text()
    declaration = re.search(r'STREAM_SUBPROTOCOL: &str = "([^"]+)"', source)
    assert declaration, GUIDANCE_RATIONALE
    expected = {declaration[1]}
    for path in STREAM_GUIDES:
        assert stream_names((ROOT / path).read_text()) == expected, (
            f"{path}: {GUIDANCE_RATIONALE}"
        )


def test_agent_shell_inventory_matches_registered_commands() -> None:
    source = (ROOT / "crates/capsem-app/src/lib.rs").read_text()
    assert documented_commands((ROOT / "AGENTS.md").read_text()) == registered_commands(source), (
        GUIDANCE_RATIONALE
    )


@pytest.mark.parametrize("stale", ["capsem.stream.v1", "capsem.stream.v2", ""])
def test_stream_guard_rejects_stale_missing_and_mixed_names(stale: str) -> None:
    current = "capsem.stream.v3"
    assert stream_names(stale) != {current}
    if stale:
        assert stream_names(current + " " + stale) != {current}


def test_shell_guard_reads_the_registered_handler_not_function_names() -> None:
    source = "fn removed() {} tauri::generate_handler![log_frontend, open_url,]"
    assert registered_commands(source) == {"log_frontend", "open_url"}
    stale = "only IPC commands are `open_url`, `check_for_app_update`."
    assert documented_commands(stale) != registered_commands(source)
    for missing in ("", "only IPC commands are `open_url`. " * 2):
        with pytest.raises(AssertionError, match="Stale architecture"):
            documented_commands(missing)
