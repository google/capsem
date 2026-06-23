from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DASHBOARD = ROOT / "frontend/src/lib/components/shell/NewTabPage.svelte"
API = ROOT / "frontend/src/lib/api.ts"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_profile_cards_are_route_owned_and_per_profile() -> None:
    source = read(DASHBOARD)

    assert "api.listProfiles()" in source
    assert "profile.availability.web" in source
    assert "function fetchProfileAssets(profile: ProfileSummary)" in source
    assert "Promise.all(profiles.map(fetchProfileAssets))" in source
    assert "getAssetsStatus(profile.id)" in source
    assert "profileAssetText(launcher.assets)" in source
    assert "profileAssetChecklist(launcher)" in source

    assert "{launcher.profile.name}" in source
    assert "{launcher.profile.description}" in source
    assert "launcher.profile.icon_svg" in source
    assert "openCustomizeProfile(launcher.profile.id)" in source
    assert "createFromProfile(launcher.profile.id)" in source
    assert "ensureProfileAssets(launcher.profile.id)" in source

    assert "Customize Session..." not in source
    assert "showCreateModal" not in source
    assert "missing profile" not in source


def test_profile_card_buttons_follow_asset_readiness() -> None:
    source = read(DASHBOARD)

    assert "launcher.assets?.ready === true" in source
    assert "ready ? createFromProfile(launcher.profile.id) : ensureProfileAssets(launcher.profile.id)" in source
    assert "ready ? `New ${launcher.profile.name} session` : profileAssetText(launcher.assets)" in source
    assert "launcher.ensuring || launcher.assets?.downloading" in source
    assert "New\n" in source
    assert "Download\n" in source
    assert "launcher.loading || launcher.creating || launcher.ensuring || launcher.assets?.downloading === true" in source


def test_profile_asset_checklist_renders_all_route_statuses() -> None:
    source = read(DASHBOARD)

    assert "VM assets" in source
    assert "asset.status === 'present'" in source
    assert "asset.status === 'downloading'" in source
    assert "<CheckCircle" in source
    assert "<CircleNotch" in source
    assert "<Warning" in source
    assert "{asset.kind ?? asset.name}" in source


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


def test_profile_and_asset_api_routes_are_profile_scoped() -> None:
    source = read(API)

    assert "export async function listProfiles" in source
    assert "export async function getAssetsStatus(profileId: string)" in source
    assert "export async function ensureAssets(profileId: string)" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/assets/status`" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/assets/ensure`" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/info`" in source
    assert "`/profiles/${encodeURIComponent(profileId)}/mcp/info`" in source
