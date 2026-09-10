// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {CapturedPayload} from "../models/CapturedPayload.js";
import {CaptureStatusSchema} from "./CaptureStatus.js";
import {CapturedContentSchema} from "./CapturedContent.js";

export const CapturedPayloadSchema: z.ZodType<CapturedPayload> = z.object({
  "content": z.lazy(() => CapturedContentSchema),
  "status": z.lazy(() => CaptureStatusSchema),
});
