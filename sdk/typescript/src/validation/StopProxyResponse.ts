// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {StopProxyResponse} from "../models/StopProxyResponse.js";

export const StopProxyResponseSchema: z.ZodType<StopProxyResponse> = z.object({
  "session_id": z.string(),
  "stopped": z.boolean(),
});
