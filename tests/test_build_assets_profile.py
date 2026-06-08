"""Profile-owned asset build rail tests."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parent.parent


def _recipe_block(name: str) -> str:
    lines = (PROJECT_ROOT / "justfile").read_text().splitlines()
    start = next(
        i
        for i, line in enumerate(lines)
        if line == name or line.startswith(f"{name} ")
    )
    end = len(lines)
    for i in range(start + 1, len(lines)):
        line = lines[i]
        if line and not line.startswith((" ", "\t", "#")):
            end = i
            break
    return "\n".join(lines[start:end])


def test_build_assets_requires_profile_and_uses_capsem_admin() -> None:
    block = _recipe_block("build-assets")

    assert 'if [[ -z "$PROFILE_ARG" ]]' in block
    assert "profile id required" in block
    assert block.index('if [[ -z "$PROFILE_ARG" ]]') < block.index("just _install-tools")
    assert "cargo run -p capsem-admin -- image build" in block
    assert '--profile "config/profiles/${PROFILE_ARG}.toml"' in block
    assert "uv run capsem-builder build guest/" not in block


def test_check_assets_recovers_with_code_profile() -> None:
    block = _recipe_block("_check-assets:")

    assert "just build-assets code" in block
