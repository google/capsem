// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PurgeRequest} from "../models/PurgeRequest.js";

export const PurgeRequestSchema: z.ZodType<PurgeRequest> = z.object({
  "all": z.boolean().exactOptional(),
});
