// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {InteractionToolResult} from "../models/InteractionToolResult.js";
import {CapturedPayloadSchema} from "./CapturedPayload.js";
import {InteractionToolResultKindSchema} from "./InteractionToolResultKind.js";

export const InteractionToolResultSchema: z.ZodType<InteractionToolResult> = z.strictObject({
  "call_id": z.string(),
  "error_message": z.string().nullable().exactOptional(),
  "is_error": z.boolean().nullable().exactOptional(),
  "kind": z.lazy(() => InteractionToolResultKindSchema),
  "payload": z.union([z.null(), z.lazy(() => CapturedPayloadSchema)]).exactOptional(),
  "response": z.union([z.null(), z.lazy(() => CapturedPayloadSchema)]).exactOptional(),
});
