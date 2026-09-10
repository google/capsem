// Generated from Capsem OpenAPI. Do not edit.

import type { UpdateCompatibilityState } from "./UpdateCompatibilityState.js";
import type { UpdateTrackState } from "./UpdateTrackState.js";

export interface UpdateTrackStatus {
  "blocked_reason"?: string | null;
  "compatibility": UpdateCompatibilityState;
  "current"?: string | null;
  "latest"?: string | null;
  "state": UpdateTrackState;
  "update_available": boolean;
}
