// Generated from Capsem OpenAPI. Do not edit.

import type { SupplyChainEvidence } from "./SupplyChainEvidence.js";
import type { UpdateTrackStatus } from "./UpdateTrackStatus.js";
import type { ValidationStatus } from "./ValidationStatus.js";

export interface UpdateStatusResponse {
  "assets": UpdateTrackStatus;
  "binary": UpdateTrackStatus;
  "channel_hash"?: string | null;
  "channel_url"?: string | null;
  "checked_at"?: number | null;
  "images": UpdateTrackStatus;
  "last_error"?: string | null;
  "profiles": UpdateTrackStatus;
  "stale": boolean;
  "supply_chain": SupplyChainEvidence;
  "validation_error"?: string | null;
  "validation_status"?: null | ValidationStatus;
}
