// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ChangesResponse} from "../models/ChangesResponse.js";
import {FileChangeSchema} from "./FileChange.js";

export const ChangesResponseSchema: z.ZodType<ChangesResponse> = z.object({
  "changes": z.array(z.lazy(() => FileChangeSchema)),
  "checkpoint": z.string(),
  "has_more": z.boolean(),
  "total": z.int().min(0),
});
