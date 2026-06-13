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
    assert '--profile "config/profiles/${PROFILE_ARG}/profile.toml"' in block
    assert "${PROFILE_ARG#profile=}" not in block
    assert "uv run capsem-builder build guest/" not in block


def test_check_assets_recovers_by_iterating_checked_in_profiles() -> None:
    block = _recipe_block("_check-assets:")

    assert 'for profile in config/profiles/*/profile.toml; do' in block
    assert 'just build-assets "$(basename "$(dirname "$profile")")" "$arch"' in block
    assert "just build-assets code" not in block


def test_runtime_recipes_materialize_generated_config_before_service() -> None:
    for recipe in ["shell:", "run-service:", "smoke:", "bench:", "install:"]:
        block = _recipe_block(recipe)
        assert "_pack-initrd" in block
        assert "_materialize-config" in block
        assert block.index("_pack-initrd") < block.index("_materialize-config")


def test_materialize_config_uses_admin_profile_command() -> None:
    block = _recipe_block("_materialize-config:")

    assert "cargo run -p capsem-admin -- profile materialize" in block
    assert "--config-root" in block
    assert "--manifest" in block
    assert "--output-root" in block
    assert "target/config" in block


def test_materialize_config_materializes_entire_checked_in_profile_catalog() -> None:
    block = _recipe_block("_materialize-config:")

    assert 'rm -rf "$ROOT/target/config"' in block
    assert 'profile_paths=("$ROOT"/config/profiles/*/profile.toml)' in block
    assert 'for profile_path in "${profile_paths[@]}"; do' in block
    assert '--profile "$profile_path"' in block
    assert '--profile "$ROOT/config/profiles/code/profile.toml"' not in block


def test_ensure_service_uses_generated_profiles() -> None:
    block = _recipe_block("_ensure-service:")

    assert 'GENERATED_PROFILES="$ROOT/target/config/profiles"' in block
    assert 'CAPSEM_PROFILES_DIR="$GENERATED_PROFILES"' in block
    assert "generated profiles missing" in block


def test_release_workflow_uses_same_config_materializer() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()

    assert workflow.count("cargo run -p capsem-admin -- profile materialize") >= 2
    assert "--output-root target/config" in workflow
    assert "--manifest assets/manifest.json" in workflow


def test_release_workflow_publishes_obom_not_debug_build_ledger() -> None:
    workflow = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text()

    assert "npm install -g @cyclonedx/cdxgen@latest" in workflow
    assert "CAPSEM_CDXGEN_CMD: cdxgen" in workflow
    assert "obom.cdx.json (arm64)" in workflow
    assert "obom.cdx.json (x86_64)" in workflow
    assert "VM base-image OBOM published" in workflow
    assert "vm-build-ledger-" not in workflow
    assert "Skipping debug-only $arch/$base from release upload" in workflow
