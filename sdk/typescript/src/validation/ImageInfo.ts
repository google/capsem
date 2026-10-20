// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageInfo} from "../models/ImageInfo.js";
import {ImageCacheStateSchema} from "./ImageCacheState.js";

export const ImageInfoSchema: z.ZodType<ImageInfo> = z.object({
  "architectures": z.array(z.string()),
  "cached": z.lazy(() => ImageCacheStateSchema),
  "description": z.string(),
  "image": z.string().nullable().exactOptional(),
  "name": z.string(),
});
