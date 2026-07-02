"""Channel switch release graph guards."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
UPDATE_RS = PROJECT_ROOT / "crates" / "capsem" / "src" / "update.rs"


def test_resolver_never_selects_revoked_manifest() -> None:
    source = UPDATE_RS.read_text(encoding="utf-8")

    assert "enum ReleaseManifestStatus" in source
    assert "ReleaseManifestStatus::Revoked" in source
    assert ".filter(|manifest| manifest.status != ReleaseManifestStatus::Revoked)" in source
    assert "channel_manifest_resolution_never_selects_revoked_manifest" in source


def test_old_capsem_selects_compatible_supported_manifest() -> None:
    source = UPDATE_RS.read_text(encoding="utf-8")

    assert "fn select_channel_manifest_url" in source
    assert "fn manifest_is_compatible_with_capsem" in source
    assert "ReleaseManifestStatus::Supported => 1" in source
    assert (
        "channel_manifest_resolution_old_capsem_selects_compatible_supported_manifest"
        in source
    )


def test_stable_and_nightly_caches_coexist() -> None:
    source = UPDATE_RS.read_text(encoding="utf-8")

    assert "fn cache_path_for_source" in source
    assert "fn cache_key_for_source" in source
    assert 'd.join("update-checks")' in source
    assert "stable_and_nightly_update_caches_coexist" in source
    assert "https://release.capsem.org/assets/nightly/manifest.json" in source
