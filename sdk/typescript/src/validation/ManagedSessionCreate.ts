// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ManagedSessionCreate} from "../models/ManagedSessionCreate.js";

export const ManagedSessionCreateSchema: z.ZodType<ManagedSessionCreate> = z.strictObject({
  "ownership_token": z.string().length(64).regex(new RegExp("^[0-9a-f]{64}$")),
  "request_id": z.uuid(),
});
