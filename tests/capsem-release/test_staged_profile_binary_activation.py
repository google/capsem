"""Contracts for staging a profile once and activating it with a later binary."""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
ADMIN = ROOT / "crates" / "capsem-admin" / "src" / "main.rs"
RELEASE_GRAPH = ROOT / "crates" / "capsem-admin" / "src" / "release_graph.rs"


def _function(source: str, name: str, next_name: str) -> str:
    return source.split(f"fn {name}", maxsplit=1)[1].split(
        f"fn {next_name}", maxsplit=1
    )[0]


def test_staged_profile_declares_minimum_and_maximum_binary_bounds() -> None:
    graph = RELEASE_GRAPH.read_text(encoding="utf-8")
    profile = graph.split("pub struct ProfileDocument", maxsplit=1)[1].split(
        "pub struct SoftwareInventoryRow", maxsplit=1
    )[0]

    assert "pub min_capsem_version: Option<String>" in profile
    assert "pub max_capsem_version: Option<String>" in profile


def test_profile_then_binary_compatibility_checks_every_current_package() -> None:
    source = ADMIN.read_text(encoding="utf-8")
    compatibility = _function(
        source,
        "graph_profile_matches_current_binary",
        "validate_graph_profiles_match_current_binary",
    )

    assert '"min_capsem_version"' in compatibility
    assert '"max_capsem_version"' in compatibility
    assert "minimum > maximum" in compatibility
    assert '== Some("current")' in compatibility
    assert "versions.iter().all" in compatibility
    assert "versions.is_empty()" in compatibility


def test_staged_profile_cannot_activate_until_binary_bounds_match() -> None:
    source = ADMIN.read_text(encoding="utf-8")
    build = _function(
        source,
        "build_assets_channel_from_graph",
        "record_graph_binary_release_metadata",
    )
    record = _function(
        source,
        "record_graph_binary_release_metadata",
        "validate_binary_release_files",
    )

    assert "validate_graph_profiles_match_current_binary(&graph_manifest)?" in build
    assert "validate_graph_profiles_match_current_binary(&manifest)?" in record
    assert record.index('manifest["packages"]') < record.index(
        "validate_graph_profiles_match_current_binary"
    )
    assert record.index("validate_graph_profiles_match_current_binary") < record.index(
        "fs::write"
    )
    assert (
        "staged_profile_then_binary_activation_enforces_bounds_without_rebuilding_profile"
        in source
    )
