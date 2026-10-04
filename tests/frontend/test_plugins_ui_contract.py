from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SETTINGS_PAGE = ROOT / "web/app/src/lib/components/shell/SettingsPage.svelte"
PLUGIN_SECTION = ROOT / "web/app/src/lib/components/settings/PluginSection.svelte"
API = ROOT / "web/app/src/lib/api.ts"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_settings_page_hosts_the_plugin_section() -> None:
    source = read(SETTINGS_PAGE)

    assert "import PluginSection from '../settings/PluginSection.svelte';" in source
    assert "{ key: 'plugins', label: 'Plugins'" in source
    assert "<PluginSection />" in source


def test_plugin_section_renders_route_owned_metadata_and_controls() -> None:
    source = read(PLUGIN_SECTION)

    assert "response = await listPlugins();" in source
    assert "credentialBrokerInfo = await getCredentialBrokerInfo();" in source
    assert "credentialBrokerInfo = await reloadCredentialBrokerStore();" in source
    assert "updatePlugin(plugin.id, { mode })" in source
    assert "updatePlugin(plugin.id, { detection_level })" in source
    assert "credentialBrokerInfo?.grants.enabled" in source

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
    assert "profile" not in source.lower()


def test_credential_rows_do_not_promote_raw_blake_refs_as_ui_identity() -> None:
    source = read(PLUGIN_SECTION)

    assert "credential.provider ?? 'Unknown provider'" in source
    assert "Last seen {credential.last_seen ?? 'never'}" in source
    assert "{credential.observed_count} seen" in source
    assert "{credential.injected_count} used" in source
    assert "{credential.credential_ref}" not in source
    assert 'font-mono text-foreground truncate">{credential.credential_ref}</p>' not in source


def test_api_exposes_only_service_wide_plugin_routes() -> None:
    source = read(API)

    assert "_get('/plugins/list')" in source
    assert "_patch(`/plugins/${encodeURIComponent(pluginId)}/edit`, update)" in source
    assert "_get('/plugins/credential_broker/credentials/info')" in source
    assert "_post('/plugins/credential_broker/credentials/reload', {})" in source
    assert "export async function listPlugins(): Promise<PluginListResponse>" in source
    assert "export async function updatePlugin(pluginId: string, update: Partial<PluginConfig>)" in source
    assert "export async function getCredentialBrokerInfo(): Promise<CredentialBrokerInfo>" in source
    assert "export async function reloadCredentialBrokerStore(): Promise<CredentialBrokerInfo>" in source
    assert "export type CredentialBrokerForkGrantDefault = 'inherit';" in source
    assert "'preprocess' | 'postprocess' | 'logging'" in source
    assert "PluginScope" not in source


def test_api_exposes_only_service_wide_mcp_routes() -> None:
    source = read(API)

    assert "_get('/mcp/servers/list')" in source
    assert "_get('/mcp/default/info')" in source
    assert "_patch('/mcp/default/edit', { action })" in source
    assert "`/mcp/servers/${encodeURIComponent(serverId)}/tools/list`" in source
    assert "`/mcp/servers/${encodeURIComponent(serverId)}/refresh`" in source
    assert "`/mcp/servers/${encodeURIComponent(serverId)}/tools/${encodeURIComponent(toolId)}/edit`" in source
    assert "`/mcp/servers/${encodeURIComponent(serverId)}/tools/${encodeURIComponent(toolId)}/call`" in source
