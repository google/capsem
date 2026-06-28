import { describe, expect, it } from 'vitest';
import { updateAvailableTracks, updateSummary, updateTrackVersion } from '../models/update-status';
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
});
