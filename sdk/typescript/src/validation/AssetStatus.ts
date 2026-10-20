// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {AssetStatus} from "../models/AssetStatus.js";
import {AssetFileStatusSchema} from "./AssetFileStatus.js";
import {AssetManifestStatusSchema} from "./AssetManifestStatus.js";

export const AssetStatusSchema: z.ZodType<AssetStatus> = z.object({
  "asset_version": z.string().nullable().exactOptional(),
  "assets": z.array(z.lazy(() => AssetFileStatusSchema)),
  "bytes_done": z.int().min(0).nullable().exactOptional(),
  "bytes_total": z.int().min(0).nullable().exactOptional(),
  "current_arch": z.string(),
  "current_asset": z.string().nullable().exactOptional(),
  "downloaded": z.int().min(0).nullable().exactOptional(),
  "downloading": z.boolean(),
  "errors": z.array(z.string()),
  "manifest": z.lazy(() => AssetManifestStatusSchema),
  "ready": z.boolean(),
  "reconcile_error": z.string().nullable().exactOptional(),
  "started": z.boolean().nullable().exactOptional(),
});
