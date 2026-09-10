// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {VmSummary} from "../models/VmSummary.js";
import {VmActionSchema} from "./VmAction.js";
import {VmLifecycleStateSchema} from "./VmLifecycleState.js";

export const VmSummarySchema: z.ZodType<VmSummary> = z.object({
  "allowed_requests": z.int().min(0).nullable().exactOptional(),
  "available_actions": z.array(z.lazy(() => VmActionSchema)),
  "can_resume": z.boolean().exactOptional(),
  "denied_requests": z.int().min(0).nullable().exactOptional(),
  "id": z.string(),
  "last_error": z.string().nullable().exactOptional(),
  "model_call_count": z.int().min(0).nullable().exactOptional(),
  "name": z.string().nullable().exactOptional(),
  "persistent": z.boolean(),
  "profile_id": z.string(),
  "resume_blocked_reason": z.string().nullable().exactOptional(),
  "status": z.lazy(() => VmLifecycleStateSchema),
  "total_estimated_cost": z.number().nullable().exactOptional(),
  "total_file_events": z.int().min(0).nullable().exactOptional(),
  "total_input_tokens": z.int().min(0).nullable().exactOptional(),
  "total_output_tokens": z.int().min(0).nullable().exactOptional(),
  "total_requests": z.int().min(0).nullable().exactOptional(),
  "total_tool_calls": z.int().min(0).nullable().exactOptional(),
  "uptime_secs": z.int().min(0).nullable().exactOptional(),
});
