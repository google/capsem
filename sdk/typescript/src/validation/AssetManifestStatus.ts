// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AssetManifestStatus} from "../models/AssetManifestStatus.js";
import {ValidationStatusSchema} from "./ValidationStatus.js";

export const AssetManifestStatusSchema: z.ZodType<AssetManifestStatus> = z.object({
  "assets_current": z.string().nullable().exactOptional(),
  "binaries_current": z.string().nullable().exactOptional(),
  "blake3": z.string().nullable().exactOptional(),
  "format": z.int().min(0).nullable().exactOptional(),
  "origin": z.string(),
  "origin_path": z.string().nullable().exactOptional(),
  "origin_source": z.string().nullable().exactOptional(),
  "packaged_at": z.string().nullable().exactOptional(),
  "path": z.string(),
  "refresh_policy": z.string().nullable().exactOptional(),
  "refreshed_at": z.string().nullable().exactOptional(),
  "validation_error": z.string().nullable().exactOptional(),
  "validation_status": z.lazy(() => ValidationStatusSchema),
});
