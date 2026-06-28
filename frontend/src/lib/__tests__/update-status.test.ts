import { describe, expect, it } from 'vitest';
import {
  updateAvailableTracks,
  updateEvidenceLinks,
  updateSummary,
  updateTrackDetail,
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

  it('distinguishes current, stale, unavailable, and not-published states', () => {
    expect(updateSummary(updateStatus())).toBe('Up to date');
    expect(updateSummary(updateStatus({ stale: true }))).toBe('Update check stale');
    expect(updateSummary(updateStatus({ last_error: 'timeout' }))).toBe('Update status unavailable');
    expect(updateTrackVersion(updateStatus().profiles)).toBe('not published');
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
