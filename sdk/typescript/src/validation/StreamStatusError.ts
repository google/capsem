// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamStatusError} from "../models/StreamStatusError.js";

export const StreamStatusErrorSchema: z.ZodType<StreamStatusError> = z.object({
  "message": z.string(),
  "type": z.literal("error"),
});
