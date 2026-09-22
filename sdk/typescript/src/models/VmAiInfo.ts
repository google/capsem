// Generated from Capsem OpenAPI. Do not edit.

import type { McpUsage } from "./McpUsage.js";
import type { ModelUsage } from "./ModelUsage.js";

export interface VmAiInfo {
  "mcp": Array<McpUsage>;
  "model_call_count": number;
  "models": Array<ModelUsage>;
  "total_estimated_cost_usd": number;
  "total_input_tokens": number;
  "total_output_tokens": number;
  "total_thinking_tokens": number;
  "total_tool_calls": number;
}
