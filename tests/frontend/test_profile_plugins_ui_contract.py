from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PROFILE_PAGE = ROOT / "frontend/src/lib/components/shell/ProfilePage.svelte"
PLUGIN_SECTION = ROOT / "frontend/src/lib/components/settings/PluginSection.svelte"
API = ROOT / "frontend/src/lib/api.ts"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_profile_page_uses_profile_scoped_plugin_and_credential_routes() -> None:
    source = read(PROFILE_PAGE)

    assert "getCredentialBrokerInfo" in source
    assert "profileSurfaces" in source
    assert "profile.profile.availability.web" in source
    assert "profile.profile.availability.shell" in source
    assert "profile.profile.availability.mobile" in source
    assert "Broker-visible credentials" in source
    assert "credentialBrokerInfo?.inventory" in source
    assert "<PluginSection {profileId} />" in source
    assert "key: 'plugins'" in source
    assert "key: 'policy'" not in source
    assert "label: 'Policy'" not in source


def test_plugin_section_renders_route_owned_metadata_and_controls() -> None:
    source = read(PLUGIN_SECTION)

    assert "listPlugins(profileId)" in source
    assert "getCredentialBrokerInfo(activeProfileId)" in source
    assert "reloadCredentialBrokerStore(activeProfileId)" in source
    assert "updatePlugin(activeProfileId, plugin.id, { mode })" in source
    assert "updatePlugin(response?.scope.profile_id ?? profileId, plugin.id, { detection_level })" in source

    assert "{plugin.name}" in source
    assert "{plugin.description}" in source
    assert "{STAGE_LABELS[plugin.stage]} · v{plugin.version}" in source
    assert "plugin.capabilities.event_families" in source
    assert "plugin.capabilities.credential_providers.join" in source
    assert "plugin.capabilities.credential_sources.join" in source
    assert "plugin.runtime.execution_count" in source
    assert "plugin.runtime.applied_count" in source
    assert "plugin.runtime.max_duration_us" in source
    assert "latency max" in source

    assert "const MODES: { value: PluginMode; label: string }[]" in source
    assert "const DETECTION_LEVELS: { value: PluginDetectionLevel; label: string }[]" in source
    assert "plugin.config.mode === 'disable'" in source
    assert "aria-label=\"{plugin.id} mode\"" in source
    assert "aria-label=\"{plugin.id} detection level\"" in source


def test_credential_rows_do_not_promote_raw_blake_refs_as_ui_identity() -> None:
    source = read(PLUGIN_SECTION)

    assert "credential.provider ?? 'Unknown provider'" in source
    assert "Last seen {credential.last_seen ?? 'never'}" in source
    assert "{credential.observed_count} seen" in source
    assert "{credential.injected_count} used" in source
    assert "{credential.credential_ref}" not in source
    assert 'font-mono text-foreground truncate">{credential.credential_ref}</p>' not in source


def test_api_exposes_only_profile_scoped_plugin_routes() -> None:
    source = read(API)

    assert "`/profiles/${encodeURIComponent(profileId)}/plugins/list`" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/plugins/${encodeURIComponent(pluginId)}/edit`" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/plugins/credential_broker/credentials/info`" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/plugins/credential_broker/credentials/reload`" in source
    assert "export async function listPlugins(profileId: string)" in source
    assert "export async function updatePlugin(" in source
    assert "export async function getCredentialBrokerInfo(profileId: string)" in source
    assert "export async function reloadCredentialBrokerStore(profileId: string)" in source
    assert "'preprocess' | 'postprocess' | 'logging'" in source
