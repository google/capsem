// Generated from Capsem OpenAPI. Do not edit.

import type { SecurityAskStatus } from "./SecurityAskStatus.js";

export interface SecurityAskRequest {
  "ask_id": string;
  "created_at_unix_ms": number;
  "event_id": string;
  "event_type": string;
  "expires_at_unix_ms": number;
  "resolution_reason"?: string | null;
  "resolved_at_unix_ms"?: number | null;
  "rule_id": string;
  "rule_name": string;
  "status": SecurityAskStatus;
  "summary": string;
}
