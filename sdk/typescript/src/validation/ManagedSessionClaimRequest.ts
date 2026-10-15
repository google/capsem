// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionClaimRequest} from "../models/ManagedSessionClaimRequest.js";

export const ManagedSessionClaimRequestSchema: z.ZodType<ManagedSessionClaimRequest> = z.strictObject({
  "ownership_token": z.string().length(64).regex(new RegExp("^[0-9a-f]{64}$")),
});
