// Generated from Capsem OpenAPI. Do not edit.

import type { ConnectionSummary } from "./ConnectionSummary.js";
import type { ScopeProfileSummary } from "./ScopeProfileSummary.js";

export interface ConnectionListResponse {
  "connections": Array<ConnectionSummary>;
  "next_cursor": string | null;
  "scope_profiles": Array<ScopeProfileSummary>;
}
