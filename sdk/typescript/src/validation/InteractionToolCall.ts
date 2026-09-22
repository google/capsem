// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionToolCall} from "../models/InteractionToolCall.js";
import {CapturedPayloadSchema} from "./CapturedPayload.js";
import {InteractionToolCallKindSchema} from "./InteractionToolCallKind.js";
import {InteractionToolResultSchema} from "./InteractionToolResult.js";
import {ToolDecisionSchema} from "./ToolDecision.js";
import {ToolOriginSchema} from "./ToolOrigin.js";

export const InteractionToolCallSchema: z.ZodType<InteractionToolCall> = z.strictObject({
  "arguments": z.union([z.null(), z.lazy(() => CapturedPayloadSchema)]).exactOptional(),
  "call_id": z.string(),
  "decision": z.lazy(() => ToolDecisionSchema),
  "kind": z.lazy(() => InteractionToolCallKindSchema),
  "origin": z.lazy(() => ToolOriginSchema),
  "request": z.union([z.null(), z.lazy(() => CapturedPayloadSchema)]).exactOptional(),
  "result": z.union([z.null(), z.lazy(() => InteractionToolResultSchema)]).exactOptional(),
  "server_name": z.string().nullable().exactOptional(),
  "tool_name": z.string(),
});
