// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {PreviewSessionResponse} from "../models/PreviewSessionResponse.js";

export const PreviewSessionResponseSchema: z.ZodType<PreviewSessionResponse> = z.object({
  "bootstrap_token": z.string(),
  "expires_in_seconds": z.int().min(0),
  "url": z.string(),
});
