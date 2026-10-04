from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DASHBOARD = ROOT / "web/app/src/lib/components/shell/NewTabPage.svelte"
API = ROOT / "web/app/src/lib/api.ts"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_new_session_launcher_follows_asset_readiness() -> None:
    source = read(DASHBOARD)

    assert "assets = await api.getAssetsStatus()" in source
    assert "assets = await api.ensureAssets()" in source
    assert "let ready = $derived(assets?.ready === true)" in source
    assert "ready ? createSession() : downloadAssets()" in source
    assert "ready ? 'New session' : assetText(assets)" in source
    assert "disabled={creatingVm || assetsLoading || downloading}" in source
    assert "vmStore.provision({ persistent: true })" in source
    assert "New\n" in source
    assert "Download\n" in source

    assert "Customize Session..." not in source
    assert "showCreateModal" not in source


def test_dashboard_does_not_render_release_diagnostics() -> None:
    source = read(DASHBOARD)

    assert ">VM assets<" not in source
    assert "Not published" not in source


def test_dashboard_groups_broken_sessions_and_exposes_refresh_and_purge() -> None:
    source = read(DASHBOARD)

    assert "healthySessions" in source
    assert "brokenSessions" in source
    assert "isBrokenSession" in source
    assert "Broken sessions" in source
    assert "Purge broken" in source
    assert "refreshDashboard" in source
    assert "handlePurgeBroken" in source
    assert "overflow-y-auto" in source
    assert "max-h-[50vh]" in source
    assert "vmStore.refresh()" in source
    assert "api.purge()" in source


def test_asset_api_routes_are_service_wide() -> None:
    source = read(API)

    assert "export async function getAssetsStatus(): Promise<AssetStatus>" in source
    assert "_sdk.call(gateway.getAssetStatus)" in source
    assert "export async function ensureAssets(): Promise<AssetStatus>" in source
    assert "_post('/assets/ensure', {})" in source
    assert "getAssetsStatus()," in source  # debugSnapshot reads /assets/status
