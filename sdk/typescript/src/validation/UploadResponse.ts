// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {UploadResponse} from "../models/UploadResponse.js";

export const UploadResponseSchema: z.ZodType<UploadResponse> = z.object({
  "container_path": z.string().nullable().exactOptional(),
  "size": z.int().min(0),
  "success": z.boolean(),
  "vm_path": z.string(),
});
