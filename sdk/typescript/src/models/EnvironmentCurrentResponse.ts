// Generated from Capsem OpenAPI. Do not edit.

import type { EnvironmentDocument } from "./EnvironmentDocument.js";

export interface EnvironmentCurrentResponse {
  "actor": string;
  "document": EnvironmentDocument;
  "reason": string;
  "request_id"?: string | null;
  "timestamp_unix_ms": number;
}
