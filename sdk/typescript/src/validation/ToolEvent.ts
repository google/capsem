// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ToolEvent} from "../models/ToolEvent.js";
import {ToolDecisionSchema} from "./ToolDecision.js";
import {ToolOriginSchema} from "./ToolOrigin.js";

export const ToolEventSchema: z.ZodType<ToolEvent> = z.object({
  "arguments": z.string().nullable().exactOptional(),
  "bytes": z.int().min(0),
  "call_id": z.string(),
  "credential_ref": z.string().nullable().exactOptional(),
  "decision": z.lazy(() => ToolDecisionSchema),
  "duration_ms": z.int().min(0),
  "error_message": z.string().nullable().exactOptional(),
  "event_id": z.string(),
  "method": z.string().nullable().exactOptional(),
  "model_call_id": z.int().nullable().exactOptional(),
  "model_parent_missing": z.boolean(),
  "process_name": z.string().nullable().exactOptional(),
  "response_preview": z.string().nullable().exactOptional(),
  "server_name": z.string(),
  "source": z.lazy(() => ToolOriginSchema),
  "timestamp": z.string().nullable().exactOptional(),
  "tool_name": z.string(),
});
