// Generated from Capsem OpenAPI. Do not edit.

import type { SessionGrantSummary } from "./SessionGrantSummary.js";

export interface SessionGrantListResponse {
  "denied_capability_ids": Array<string>;
  "grants": Array<SessionGrantSummary>;
  "owner_generation": string;
  "session_id": string;
}
