// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmAiInfo} from "../models/VmAiInfo.js";
import {McpUsageSchema} from "./McpUsage.js";
import {ModelUsageSchema} from "./ModelUsage.js";

export const VmAiInfoSchema: z.ZodType<VmAiInfo> = z.object({
  "mcp": z.array(z.lazy(() => McpUsageSchema)),
  "model_call_count": z.int().min(0),
  "models": z.array(z.lazy(() => ModelUsageSchema)),
  "total_estimated_cost_usd": z.number(),
  "total_input_tokens": z.int().min(0),
  "total_output_tokens": z.int().min(0),
  "total_thinking_tokens": z.int().min(0),
  "total_tool_calls": z.int().min(0),
});
