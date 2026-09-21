// Generated from Capsem OpenAPI. Do not edit.

import type { VmAction } from "./VmAction.js";
import type { VmLifecycleState } from "./VmLifecycleState.js";

export interface VmSummary {
  "allowed_requests"?: number | null;
  "available_actions": Array<VmAction>;
  "can_resume"?: boolean;
  "denied_requests"?: number | null;
  "id": string;
  "last_error"?: string | null;
  "model_call_count"?: number | null;
  "name"?: string | null;
  "persistent": boolean;
  "profile_id": string;
  "resume_blocked_reason"?: string | null;
  "status": VmLifecycleState;
  "total_estimated_cost"?: number | null;
  "total_file_events"?: number | null;
  "total_input_tokens"?: number | null;
  "total_output_tokens"?: number | null;
  "total_requests"?: number | null;
  "total_tool_calls"?: number | null;
  "uptime_secs"?: number | null;
}
