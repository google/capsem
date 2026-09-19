// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SessionDbStatus} from "../models/SessionDbStatus.js";

export const SessionDbStatusSchema: z.ZodType<SessionDbStatus> = z.object({
  "error": z.string().nullable().exactOptional(),
  "ready": z.boolean(),
});
