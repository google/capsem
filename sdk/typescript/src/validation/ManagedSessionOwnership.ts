// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionOwnership} from "../models/ManagedSessionOwnership.js";

export const ManagedSessionOwnershipSchema: z.ZodType<ManagedSessionOwnership> = z.strictObject({
  "expires_at": z.string(),
  "generation": z.string(),
  "lease_seconds": z.int().min(0),
  "renew_after_seconds": z.int().min(0),
  "request_id": z.string(),
});
