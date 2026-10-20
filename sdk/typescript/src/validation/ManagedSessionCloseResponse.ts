// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionCloseResponse} from "../models/ManagedSessionCloseResponse.js";
import {ManagedSessionClosedStateSchema} from "./ManagedSessionClosedState.js";

export const ManagedSessionCloseResponseSchema: z.ZodType<ManagedSessionCloseResponse> = z.strictObject({
  "evidence_retained": z.boolean(),
  "generation": z.string(),
  "request_id": z.string(),
  "session_id": z.string().nullable().exactOptional(),
  "state": z.lazy(() => ManagedSessionClosedStateSchema),
});
