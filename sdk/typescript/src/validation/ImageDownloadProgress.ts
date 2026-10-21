// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ImageDownloadProgress} from "../models/ImageDownloadProgress.js";
import {ImageDownloadPhaseSchema} from "./ImageDownloadPhase.js";

export const ImageDownloadProgressSchema: z.ZodType<ImageDownloadProgress> = z.object({
  "completed_bytes": z.int().min(0),
  "phase": z.lazy(() => ImageDownloadPhaseSchema),
  "total_bytes": z.int().min(0).nullable().exactOptional(),
});
