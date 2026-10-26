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
    assert "<ImageCatalog {ready} {prepareRuntime} />" in source
    assert "if (ready) return;" in source
    assert "while (assets.downloading && Date.now() < deadline)" in source
    catalog = read(DASHBOARD.with_name("ImageCatalog.svelte"))
    assert "await startImageSession(image, ready," in catalog
    assert "prepareRuntime," in catalog
    assert "vmStore.provision(imageLaunchRequest(image))" in catalog
    assert "disabled={caching !== null || launching !== null || vmStore.acting}" in catalog
    launch = read(ROOT / "web/app/src/lib/models/image-launch.ts")
    assert "if (!runtimeReady) await actions.prepareRuntime();" in launch
    assert "await actions.launch();" in launch
    assert "return { persistent: true, container:" in launch

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
    assert "watchVisibleRefresh(async () =>" in source
    assert "await Promise.all([loadAssets(), loadStats()]);" in source
    assert "handlePurgeBroken" in source
    assert "overflow-y-auto" in source
    assert "max-h-[50vh]" in source
    assert "vmStore.refresh()" in source
    assert "const ids = await vmStore.deleteBroken();" in source
    assert "ids.includes(tab.vmId)" in source


def test_asset_api_routes_are_service_wide() -> None:
    source = read(API)

    assert "export async function getAssetsStatus(): Promise<AssetStatus>" in source
    assert "_sdk.call(gateway.getAssetStatus)" in source
    assert "export async function ensureAssets(): Promise<AssetStatus>" in source
    assert "_post('/assets/ensure', {})" in source
    assert "getAssetsStatus()," in source  # debugSnapshot reads /assets/status
