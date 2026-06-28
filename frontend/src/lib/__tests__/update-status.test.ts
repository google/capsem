import { describe, expect, it } from 'vitest';
import {
  profileDashboardUpdateRows,
  profileDashboardUpdateSummary,
  updateAvailableTracks,
  updateBlockedTracks,
  updateEvidenceLinks,
  updateSummary,
  updateTrackDetail,
  updateTrackStateLabel,
  updateTrackTone,
  updateTrackVersion,
} from '../models/update-status';
import type { UpdateStatusResponse, UpdateTrackStatus } from '../types/gateway';

function currentTrack(current = '1.4.0'): UpdateTrackStatus {
  return {
    current,
    latest: current,
    update_available: false,
    state: 'current',
    compatibility: 'compatible',
  };
}

function updateStatus(patch: Partial<UpdateStatusResponse> = {}): UpdateStatusResponse {
  return {
    checked_at: 1718444400,
    channel_url: 'https://release.capsem.org/health.json',
    stale: false,
    binary: currentTrack(),
    assets: currentTrack('assets-1'),
    profiles: {
      update_available: false,
      state: 'not_published',
      compatibility: 'not_applicable',
    },
    images: {
      update_available: false,
      state: 'not_published',
      compatibility: 'not_applicable',
    },
    ...patch,
  };
}

describe('update status model', () => {
  it('summarizes available binary and asset updates', () => {
    const status = updateStatus({
      binary: {
        current: '1.4.0',
        latest: '1.4.1',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
      assets: {
        current: 'assets-1',
        latest: 'assets-2',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
      images: {
        current: 'images-1',
        latest: 'images-2',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
    });

    expect(updateAvailableTracks(status)).toEqual(['binary', 'assets', 'images']);
    expect(updateSummary(status)).toBe('Binary, VM assets, VM images available');
    expect(updateTrackVersion(status.binary)).toBe('1.4.0 -> 1.4.1');
  });

  it('treats profile catalog updates as a first-class available track', () => {
    const status = updateStatus({
      profiles: {
        current: 'profiles-2030.0101.0',
        latest: 'profiles-2030.0101.1',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
    });

    expect(updateAvailableTracks(status)).toEqual(['profiles']);
    expect(updateSummary(status)).toBe('Profiles available');
    expect(updateTrackVersion(status.profiles)).toBe(
      'profiles-2030.0101.0 -> profiles-2030.0101.1',
    );
    expect(updateTrackStateLabel(status.profiles)).toBe('Update available');
    expect(updateTrackTone(status.profiles)).toBe('available');
  });

  it('builds profile dashboard rows without treating binary updates as profile state', () => {
    const status = updateStatus({
      binary: {
        current: '1.4.0',
        latest: '1.4.1',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
      profiles: {
        current: 'profiles-2030.0101.0',
        latest: 'profiles-2030.0101.1',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
      assets: {
        current: '2030.0101.0',
        latest: '2030.0101.1',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
    });

    expect(profileDashboardUpdateSummary(status)).toBe(
      'Profiles, VM assets available for future sessions',
    );
    expect(profileDashboardUpdateRows(status).map(row => row.key)).toEqual([
      'profiles',
      'assets',
      'images',
    ]);
    expect(profileDashboardUpdateRows(status).map(row => row.label)).not.toContain('Binary');
    expect(profileDashboardUpdateRows(status)[0].detail).toContain('existing sessions stay pinned');
    expect(profileDashboardUpdateRows(status)[1].detail).toContain('capsem update --assets');
  });

  it('surfaces blocked profile state on the profile dashboard', () => {
    const status = updateStatus({
      profiles: {
        current: 'profiles-2030.0101.0',
        latest: 'profiles-2030.0101.1',
        update_available: false,
        state: 'current',
        compatibility: 'compatible',
        blocked_reason: 'requires binary 1.4.1 or newer',
      },
    });

    expect(profileDashboardUpdateSummary(status)).toBe('Profiles blocked');
    const [profiles] = profileDashboardUpdateRows(status);
    expect(profiles.stateLabel).toBe('Blocked');
    expect(profiles.tone).toBe('blocked');
    expect(profiles.detail).toBe('requires binary 1.4.1 or newer');
  });

  it('keeps blocked tracks visible when another update is available', () => {
    const status = updateStatus({
      binary: {
        current: '1.4.0',
        latest: '1.4.1',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
      profiles: {
        current: 'profiles-2030.0101.0',
        latest: 'profiles-2030.0101.1',
        update_available: false,
        state: 'current',
        compatibility: 'compatible',
        blocked_reason: 'requires binary 1.4.1 or newer',
      },
    });

    expect(updateAvailableTracks(status)).toEqual(['binary']);
    expect(updateBlockedTracks(status)).toEqual(['profiles']);
    expect(updateSummary(status)).toBe('Binary available; Profiles blocked');
    expect(profileDashboardUpdateSummary(status)).toBe('Profiles blocked');
  });

  it('surfaces blocked VM asset updates on the profile dashboard', () => {
    const status = updateStatus({
      assets: {
        current: '2026.0627.1',
        latest: '2030.0101.1',
        update_available: false,
        state: 'unknown',
        compatibility: 'unknown',
        blocked_reason: 'requires binary 99.99.99 or newer',
      },
    });

    expect(updateBlockedTracks(status)).toEqual(['assets']);
    expect(updateSummary(status)).toBe('VM assets blocked');
    expect(profileDashboardUpdateSummary(status)).toBe('VM assets blocked');
    const [, assets] = profileDashboardUpdateRows(status);
    expect(assets.stateLabel).toBe('Blocked');
    expect(assets.tone).toBe('blocked');
    expect(assets.detail).toBe('requires binary 99.99.99 or newer');
  });

  it('keeps blocked profile dashboard tracks visible beside available asset tracks', () => {
    const status = updateStatus({
      assets: {
        current: 'assets-1',
        latest: 'assets-2',
        update_available: true,
        state: 'update_available',
        compatibility: 'compatible',
      },
      profiles: {
        current: 'profiles-2030.0101.0',
        latest: 'profiles-2030.0101.1',
        update_available: false,
        state: 'current',
        compatibility: 'compatible',
        blocked_reason: 'requires binary 1.4.1 or newer',
      },
    });

    expect(profileDashboardUpdateSummary(status)).toBe(
      'VM assets available for future sessions; Profiles blocked',
    );
  });

  it('distinguishes current, stale, unavailable, and not-published states', () => {
    expect(updateSummary(updateStatus())).toBe('Up to date');
    expect(updateSummary(updateStatus({ stale: true }))).toBe('Update check stale');
    expect(updateSummary(updateStatus({ last_error: 'timeout' }))).toBe('Update status unavailable');
    expect(updateTrackVersion(updateStatus().profiles)).toBe('not published');
    expect(updateTrackStateLabel(currentTrack())).toBe('Current');
    expect(updateTrackStateLabel(updateStatus().profiles)).toBe('Not published');
    expect(updateTrackStateLabel({
      update_available: false,
      state: 'unknown',
      compatibility: 'unknown',
    })).toBe('Unknown');
  });

  it('surfaces blocked track details without inventing detail for normal states', () => {
    expect(updateTrackDetail(currentTrack())).toBeNull();
    expect(updateTrackDetail({
      ...currentTrack(),
      compatibility: 'unknown',
    })).toBe('Compatibility unknown');
    expect(updateTrackDetail({
      ...currentTrack(),
      blocked_reason: 'requires binary 1.4.1',
    })).toBe('requires binary 1.4.1');
    expect(updateTrackStateLabel({
      ...currentTrack(),
      blocked_reason: 'requires binary 1.4.1',
    })).toBe('Blocked');
    expect(updateTrackTone({
      ...currentTrack(),
      blocked_reason: 'requires binary 1.4.1',
    })).toBe('blocked');
    expect(updateTrackTone(currentTrack())).toBe('muted');
  });

  it('extracts release evidence links from supply-chain status', () => {
    const status = updateStatus({
      supply_chain: {
        manifest: {
          origin: 'https://release.capsem.org/assets/stable/manifest.json',
          path: '/Users/me/.capsem/assets/manifest.json',
          blake3: 'manifest-hash',
        },
        channel_index: {
          url: 'https://release.capsem.org/health.json',
          blake3: 'channel-hash',
        },
        host_sbom: {
          name: 'host-sbom',
          format: 'spdx',
          scope: 'binary',
          route: '/release/sbom/host.spdx.json',
        },
        vm_obom: {
          name: 'vm-obom',
          format: 'cyclonedx',
          scope: 'vm_assets',
          release_artifact: '/assets/releases/assets-1/arm64-obom.cdx.json',
        },
        attestations: [
          {
            name: 'binary provenance',
            scope: 'binary',
            workflow: '.github/workflows/release.yaml',
            release_artifact: 'https://github.com/capsem/releases/download/v1/capsem.pkg.intoto.jsonl',
          },
          {
            name: 'vm asset provenance',
            scope: 'vm_assets',
            workflow: '.github/workflows/release-assets.yaml',
            release_artifact: '/assets/releases/assets-1/arm64-rootfs.erofs.intoto.jsonl',
          },
        ],
      },
    });

    expect(updateEvidenceLinks(status)).toEqual([
      {
        label: 'Host SBOM',
        href: '/release/sbom/host.spdx.json',
        meta: 'spdx · binary',
      },
      {
        label: 'VM OBOM',
        href: '/assets/releases/assets-1/arm64-obom.cdx.json',
        meta: 'cyclonedx · vm_assets',
      },
      {
        label: 'Binary attestation',
        href: 'https://github.com/capsem/releases/download/v1/capsem.pkg.intoto.jsonl',
        meta: 'binary',
      },
      {
        label: 'VM asset attestation',
        href: '/assets/releases/assets-1/arm64-rootfs.erofs.intoto.jsonl',
        meta: 'vm_assets',
      },
    ]);
  });
});
