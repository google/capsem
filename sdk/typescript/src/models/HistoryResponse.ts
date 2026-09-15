// Generated from Capsem OpenAPI. Do not edit.

import type { HistoryEntry } from "./HistoryEntry.js";

export interface HistoryResponse {
  "commands": Array<HistoryEntry>;
  "has_more": boolean;
  "total": number;
}
