// Generated from Capsem OpenAPI. Do not edit.

import type { ToolDecision } from "./ToolDecision.js";
import type { ToolOrigin } from "./ToolOrigin.js";

export interface ToolEvent {
  "arguments"?: string | null;
  "bytes": number;
  "call_id": string;
  "credential_ref"?: string | null;
  "decision": ToolDecision;
  "duration_ms": number;
  "error_message"?: string | null;
  "event_id": string;
  "method"?: string | null;
  "model_call_id"?: number | null;
  "model_parent_missing": boolean;
  "process_name"?: string | null;
  "response_preview"?: string | null;
  "server_name": string;
  "source": ToolOrigin;
  "timestamp"?: string | null;
  "tool_name": string;
}
