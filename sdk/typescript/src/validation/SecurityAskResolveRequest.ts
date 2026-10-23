// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {SecurityAskResolveRequest} from "../models/SecurityAskResolveRequest.js";
import {SecurityAskDecisionSchema} from "./SecurityAskDecision.js";

export const SecurityAskResolveRequestSchema: z.ZodType<SecurityAskResolveRequest> = z.strictObject({
  "decision": z.lazy(() => SecurityAskDecisionSchema),
  "generation": z.string(),
});
