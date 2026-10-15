// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionCloseRequest} from "../models/ManagedSessionCloseRequest.js";
import {ManagedSessionOutcomeSchema} from "./ManagedSessionOutcome.js";

export const ManagedSessionCloseRequestSchema: z.ZodType<ManagedSessionCloseRequest> = z.strictObject({
  "outcome": z.lazy(() => ManagedSessionOutcomeSchema),
  "ownership_token": z.string().length(64).regex(new RegExp("^[0-9a-f]{64}$")),
});
