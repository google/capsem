// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestDecisionDetails} from "../models/RequestDecisionDetails.js";

export const RequestDecisionDetailsSchema: z.ZodType<RequestDecisionDetails> = z.strictObject({
  "actor": z.string(),
  "reason": z.string(),
  "timestamp_unix_ms": z.int(),
});
