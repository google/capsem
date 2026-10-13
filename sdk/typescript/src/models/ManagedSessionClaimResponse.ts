// Generated from Capsem OpenAPI. Do not edit.

import type { ManagedSessionFailure } from "./ManagedSessionFailure.js";
import type { ManagedSessionOwnership } from "./ManagedSessionOwnership.js";
import type { ManagedSessionState } from "./ManagedSessionState.js";
import type { ProvisionResponse } from "./ProvisionResponse.js";

export interface ManagedSessionClaimResponse {
  "error"?: null | ManagedSessionFailure;
  "ownership": ManagedSessionOwnership;
  "session"?: null | ProvisionResponse;
  "state": ManagedSessionState;
}
