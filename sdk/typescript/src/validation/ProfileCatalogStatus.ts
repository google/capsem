// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import type {ProfileCatalogStatus} from "../models/ProfileCatalogStatus.js";
import {AssetManifestStatusSchema} from "./AssetManifestStatus.js";
import {ProfileCatalogSourceSchema} from "./ProfileCatalogSource.js";
import {ProfileReadinessSchema} from "./ProfileReadiness.js";

export const ProfileCatalogStatusSchema: z.ZodType<ProfileCatalogStatus> = z.object({
  "asset_manifest": z.union([z.null(), z.lazy(() => AssetManifestStatusSchema)]).exactOptional(),
  "bytes_done": z.int().min(0).nullable().exactOptional(),
  "bytes_total": z.int().min(0).nullable().exactOptional(),
  "current_asset": z.string().nullable().exactOptional(),
  "downloaded": z.int().min(0).nullable().exactOptional(),
  "profile_count": z.int().min(0),
  "profiles": z.array(z.lazy(() => ProfileReadinessSchema)),
  "ready_count": z.int().min(0),
  "reconcile_error": z.string().nullable().exactOptional(),
  "source": z.lazy(() => ProfileCatalogSourceSchema),
});
