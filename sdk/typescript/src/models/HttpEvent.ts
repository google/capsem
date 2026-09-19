// Generated from Capsem OpenAPI. Do not edit.

import type { NetworkDecision } from "./NetworkDecision.js";

export interface HttpEvent {
  "bytes_received"?: number | null;
  "bytes_sent"?: number | null;
  "credential_ref"?: string | null;
  "decision": NetworkDecision;
  "domain": string;
  "duration_ms"?: number | null;
  "event_id": string;
  "matched_rule"?: string | null;
  "method"?: string | null;
  "path"?: string | null;
  "policy_rule"?: string | null;
  "port"?: number | null;
  "query"?: string | null;
  "request_headers"?: string | null;
  "response_headers"?: string | null;
  "status_code"?: number | null;
  "timestamp": string;
  "trace_id"?: string | null;
}
