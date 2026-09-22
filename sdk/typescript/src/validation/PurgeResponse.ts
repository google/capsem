// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PurgeResponse} from "../models/PurgeResponse.js";

export const PurgeResponseSchema: z.ZodType<PurgeResponse> = z.object({
  "ephemeral_purged": z.int().min(0),
  "persistent_purged": z.int().min(0),
  "purged": z.int().min(0),
});
