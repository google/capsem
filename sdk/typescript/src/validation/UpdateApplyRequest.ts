// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UpdateApplyRequest} from "../models/UpdateApplyRequest.js";

export const UpdateApplyRequestSchema: z.ZodType<UpdateApplyRequest> = z.strictObject({
  "confirmed": z.boolean().exactOptional(),
  "dry_run": z.boolean().exactOptional(),
});
