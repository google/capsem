// Generated from Capsem OpenAPI. Do not edit.

import type { ManagedSessionClosedState } from "./ManagedSessionClosedState.js";

export interface ManagedSessionCloseResponse {
  "evidence_retained": boolean;
  "generation": string;
  "request_id": string;
  "session_id"?: string | null;
  "state": ManagedSessionClosedState;
}
