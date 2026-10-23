// Generated from Capsem OpenAPI. Do not edit.

import type { BrowserRequest } from "./BrowserRequest.js";
import type { CostCoverage } from "./CostCoverage.js";
import type { ImageApp } from "./ImageApp.js";
import type { TerminalSummary } from "./TerminalSummary.js";
import type { VmAction } from "./VmAction.js";
import type { VmLifecycleState } from "./VmLifecycleState.js";

export interface VmSummary {
  "allowed_requests"?: number | null;
  "app"?: null | ImageApp;
  "available_actions": Array<VmAction>;
  "browser_request"?: null | BrowserRequest;
  "can_resume"?: boolean;
  "cost_coverage"?: null | CostCoverage;
  "denied_requests"?: number | null;
  "id": string;
  "last_error"?: string | null;
  "model_call_count"?: number | null;
  "name"?: string | null;
  "pending_approval_count"?: number;
  "persistent": boolean;
  "resume_blocked_reason"?: string | null;
  "status": VmLifecycleState;
  "terminal"?: null | TerminalSummary;
  "total_estimated_cost"?: number | null;
  "total_file_events"?: number | null;
  "total_input_tokens"?: number | null;
  "total_output_tokens"?: number | null;
  "total_requests"?: number | null;
  "total_tool_calls"?: number | null;
  "uptime_secs"?: number | null;
}
