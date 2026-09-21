// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ErrorEvent} from "../models/ErrorEvent.js";

export const ErrorEventSchema: z.ZodType<ErrorEvent> = z.object({
  "binary": z.string(),
  "level": z.string(),
  "message": z.string(),
  "target": z.string().nullable().exactOptional(),
  "ts": z.string(),
});
