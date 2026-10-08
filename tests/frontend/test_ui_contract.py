from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FRONTEND = ROOT / "web/app/src"


def read(relative: str) -> str:
    return (FRONTEND / relative).read_text(encoding="utf-8")


def test_frontend_uses_current_route_vocabulary_not_retired_policy_vm_terms() -> None:
    dashboard = read("lib/components/shell/NewTabPage.svelte")
    session_errors = read("lib/models/session-action-error.ts")
    settings = read("lib/components/shell/SettingsPage.svelte")
    stats = read("lib/components/views/StatsView.svelte")
    toolbar = read("lib/components/shell/Toolbar.svelte")

    assert "Sessions" in dashboard
    assert "sessionActionError(error)" in dashboard
    assert "Failed to create session" in session_errors
    assert "Session {vmId} ledger" in stats
    assert "Session Logs" in toolbar

    combined = "\n".join([dashboard, settings, stats, toolbar])
    assert ">VMs<" not in combined
    assert "Customize VM" not in combined
    assert "label: 'Policy'" not in combined
    assert "key: 'policy'" not in combined
    assert "Frontend build" not in combined
    assert "build {__BUILD_TS__}" not in combined


def test_settings_page_hosts_service_wide_plugins_and_mcp() -> None:
    source = read("lib/components/shell/SettingsPage.svelte")

    assert "{ key: 'plugins', label: 'Plugins'" in source
    assert "{ key: 'mcp', label: 'MCP'" in source
    assert "<PluginSection />" in source
    assert "<McpSection />" in source


def test_detail_panes_render_one_canonical_payload_view_without_preview_duplicates() -> None:
    source = read("lib/components/views/StatsView.svelte")

    assert "bodyBlobs = detailRows.body_blobs" in source
    assert "event_body_blobs" not in source
    assert "showDetail" in source
    assert "detailPayloadSections" in source
    assert "visibleDetailEntries" in source
    assert "codeToHtml" in source

    assert "response_body_preview" not in source
    assert "request_body_preview" not in source
    assert "JSON.stringify(detail" not in source
    assert "credential:blake3" not in source


def test_ui_chrome_uses_semantic_tokens_not_raw_status_colors() -> None:
    source_files = [
        read("lib/components/shell/Toolbar.svelte"),
        read("lib/components/shell/NewTabPage.svelte"),
        read("lib/components/settings/PluginSection.svelte"),
        read("lib/components/settings/McpSection.svelte"),
        read("lib/components/views/StatsView.svelte"),
    ]
    combined = "\n".join(source_files)

    assert "bg-primary" in combined
    assert "text-primary" in combined
    assert "text-destructive" in combined
    assert "bg-green-" not in combined
    assert "text-green-" not in combined
    assert "bg-red-" not in combined
    assert "text-red-" not in combined
    assert "bg-amber-" not in combined
    assert "text-amber-" not in combined
