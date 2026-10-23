import { UpdateCompatibilityState, UpdateTrackState, ValidationStatus, type UpdateStatusResponse } from '@capsem/sdk';

/** The service owns release validation and version ordering. */
export function capsemUpdateVersion(status: UpdateStatusResponse | null): string | null {
  if (!status || status.stale || status.last_error || status.validation_error ||
      status.validation_status !== ValidationStatus.VALID) return null;
  const binary = status.binary;
  if (!binary.update_available || binary.state !== UpdateTrackState.UPDATE_AVAILABLE ||
      binary.compatibility !== UpdateCompatibilityState.COMPATIBLE || binary.blocked_reason ||
      !binary.current || !binary.latest || binary.latest === binary.current) return null;
  return binary.latest;
}
