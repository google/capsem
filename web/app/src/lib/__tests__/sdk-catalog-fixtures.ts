import { ProfileExistingVmUpdateSemantics, ProfileNewSessionUpdateSemantics, ProfileUpgradeAction,
  UpdateCompatibilityState, UpdateTrackState, type ProfileSummary, type UpdateStatusResponse } from '@capsem/sdk';

export const profile: ProfileSummary = {
  id: 'code', name: 'Code', description: 'SDK fixture', source: 'profile', icon_svg: null,
  availability: { web: true, shell: true, mobile: false }, rule_count: 1, default_rule_count: 1,
  plugin_count: 0, mcp_server_count: 0,
  update_semantics: {
    existing_vms: ProfileExistingVmUpdateSemantics.PINNED_UNTIL_RECREATE,
    new_sessions: ProfileNewSessionUpdateSemantics.USE_CURRENT_PROFILE_CATALOG,
    upgrade_action: ProfileUpgradeAction.RECREATE_VM,
  },
};

export function updateStatusFixture(): UpdateStatusResponse {
  const available = { current: '1.4.0', latest: '1.4.1', update_available: true,
    state: UpdateTrackState.UPDATE_AVAILABLE, compatibility: UpdateCompatibilityState.COMPATIBLE };
  const absent = { update_available: false, state: UpdateTrackState.NOT_PUBLISHED,
    compatibility: UpdateCompatibilityState.NOT_APPLICABLE };
  return {
    checked_at: 1718444400, channel_url: 'https://release.capsem.org/assets/stable/manifest.json', stale: false,
    binary: available, assets: { ...available, current: 'assets-1', latest: 'assets-2' },
    profiles: absent, images: { ...absent },
    supply_chain: { manifest: { path: 'manifest.json' }, channel_index: {},
      host_sbom: { name: 'host', route: '/sbom.json' }, vm_obom: { name: 'vm' }, attestations: [] },
  };
}
