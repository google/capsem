// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamStatusExit} from "../models/StreamStatusExit.js";

export const StreamStatusExitSchema: z.ZodType<StreamStatusExit> = z.object({
  "code": z.int(),
  "truncated": z.boolean(),
  "type": z.literal("exit"),
});
