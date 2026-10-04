// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImagePullResponse} from "../models/ImagePullResponse.js";

export const ImagePullResponseSchema: z.ZodType<ImagePullResponse> = z.object({
  "digest": z.string(),
  "image": z.string(),
  "resolved": z.string(),
});
