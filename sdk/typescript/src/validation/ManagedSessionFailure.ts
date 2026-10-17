// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionFailure} from "../models/ManagedSessionFailure.js";
import {ManagedSessionErrorCodeSchema} from "./ManagedSessionErrorCode.js";

export const ManagedSessionFailureSchema: z.ZodType<ManagedSessionFailure> = z.strictObject({
  "code": z.lazy(() => ManagedSessionErrorCodeSchema),
  "error": z.string(),
});
