// Generated from Capsem OpenAPI. Do not edit.

import type { CostCoverage } from "./CostCoverage.js";

export interface VmStatsSummaryResponse {
  "allowed_requests": number;
  "cost_coverage"?: null | CostCoverage;
  "denied_requests": number;
  "total_estimated_cost": number;
  "total_input_tokens": number;
  "total_output_tokens": number;
  "total_requests": number;
  "total_thinking_tokens": number;
  "total_tool_calls": number;
}
