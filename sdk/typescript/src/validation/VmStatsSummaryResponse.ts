// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmStatsSummaryResponse} from "../models/VmStatsSummaryResponse.js";

export const VmStatsSummaryResponseSchema: z.ZodType<VmStatsSummaryResponse> = z.object({
  "allowed_requests": z.int().min(0),
  "denied_requests": z.int().min(0),
  "total_estimated_cost": z.number(),
  "total_input_tokens": z.int().min(0),
  "total_output_tokens": z.int().min(0),
  "total_requests": z.int().min(0),
  "total_thinking_tokens": z.int().min(0),
  "total_tool_calls": z.int().min(0),
});
