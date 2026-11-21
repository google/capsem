// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {RequestResolveRequest} from "../models/RequestResolveRequest.js";
import {RequestDecisionSchema} from "./RequestDecision.js";

export const RequestResolveRequestSchema: z.ZodType<RequestResolveRequest> = z.strictObject({
  "decision": z.lazy(() => RequestDecisionSchema),
  "observed_revision": z.string(),
  "reason": z.string(),
});
