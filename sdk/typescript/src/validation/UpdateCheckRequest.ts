// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UpdateCheckRequest} from "../models/UpdateCheckRequest.js";

export const UpdateCheckRequestSchema: z.ZodType<UpdateCheckRequest> = z.strictObject({
  "dry_run": z.boolean().exactOptional(),
});
