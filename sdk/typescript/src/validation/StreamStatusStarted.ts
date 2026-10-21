// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StreamStatusStarted} from "../models/StreamStatusStarted.js";

export const StreamStatusStartedSchema: z.ZodType<StreamStatusStarted> = z.object({
  "type": z.literal("started"),
});
