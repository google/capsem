// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageInfo} from "../models/ImageInfo.js";
import {ImageAppSchema} from "./ImageApp.js";
import {ImageCacheStateSchema} from "./ImageCacheState.js";
import {ImageDownloadProgressSchema} from "./ImageDownloadProgress.js";
import {ImageSurfaceSchema} from "./ImageSurface.js";

export const ImageInfoSchema: z.ZodType<ImageInfo> = z.object({
  "app": z.union([z.null(), z.lazy(() => ImageAppSchema)]).exactOptional(),
  "architectures": z.array(z.string()),
  "cached": z.lazy(() => ImageCacheStateSchema),
  "description": z.string(),
  "image": z.string().nullable().exactOptional(),
  "name": z.string(),
  "previous_image": z.string().nullable().exactOptional(),
  "progress": z.union([z.null(), z.lazy(() => ImageDownloadProgressSchema)]).exactOptional(),
  "surface": z.union([z.null(), z.lazy(() => ImageSurfaceSchema)]).exactOptional(),
});
