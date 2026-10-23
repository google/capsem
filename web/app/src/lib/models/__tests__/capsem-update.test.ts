import { describe, expect, it } from 'vitest';
import { UpdateCompatibilityState, UpdateTrackState, ValidationStatus } from '@capsem/sdk';
import { updateStatusFixture } from '../../__tests__/sdk-catalog-fixtures';
import { capsemUpdateVersion } from '../capsem-update';

describe('Capsem update notice', () => {
  it('uses a fresh validated service-owned newer binary signal', () => {
    const status = { ...updateStatusFixture(), validation_status: ValidationStatus.VALID };
    expect(capsemUpdateVersion(status)).toBe('1.4.1');
  });

  it('does not claim updates from stale, failed, missing or blocked checks', () => {
    const status = { ...updateStatusFixture(), validation_status: ValidationStatus.VALID };
    expect(capsemUpdateVersion(null)).toBeNull();
    for (const change of [{ stale: true }, { last_error: 'offline' },
      { validation_status: ValidationStatus.INVALID }, { validation_status: undefined }]) {
      expect(capsemUpdateVersion({ ...status, ...change })).toBeNull();
    }
    for (const change of [{ update_available: false }, { latest: undefined },
      { current: undefined }, { latest: status.binary.current }, { blocked_reason: 'runtime unsupported' },
      { state: UpdateTrackState.UNKNOWN }, { compatibility: UpdateCompatibilityState.UNKNOWN }]) {
      expect(capsemUpdateVersion({ ...status, binary: { ...status.binary, ...change } })).toBeNull();
    }
  });

  it('does not substitute runtime images for a Capsem version update', () => {
    const status = { ...updateStatusFixture(), validation_status: ValidationStatus.VALID };
    expect(capsemUpdateVersion({ ...status, binary: { ...status.binary, update_available: false },
      images: status.assets })).toBeNull();
  });
});
