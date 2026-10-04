import { UpdateCompatibilityState, UpdateTrackState, type UpdateStatusResponse } from '@capsem/sdk';

export function updateStatusFixture(): UpdateStatusResponse {
  const available = { current: '1.4.0', latest: '1.4.1', update_available: true,
    state: UpdateTrackState.UPDATE_AVAILABLE, compatibility: UpdateCompatibilityState.COMPATIBLE };
  const absent = { update_available: false, state: UpdateTrackState.NOT_PUBLISHED,
    compatibility: UpdateCompatibilityState.NOT_APPLICABLE };
  return {
    checked_at: 1718444400, channel_url: 'https://release.capsem.org/assets/stable/manifest.json', stale: false,
    binary: available, assets: { ...available, current: 'assets-1', latest: 'assets-2' },
    images: absent,
    supply_chain: { manifest: { path: 'manifest.json' }, channel_index: {},
      host_sbom: { name: 'host', route: '/sbom.json' }, vm_obom: { name: 'vm_obom' }, attestations: [] },
  };
}
