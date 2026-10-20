// Generated from Capsem OpenAPI. Do not edit.

import type { GrantState } from "./GrantState.js";

export interface SessionGrantSummary {
  "capability_ids": Array<string>;
  "connection_id": string;
  "created_at": string;
  "expires_at": string;
  "id": string;
  "owner_generation": string;
  "revision": number;
  "revoked_at": string | null;
  "runtime_bound": boolean;
  "session_id": string;
  "state": GrantState;
}
