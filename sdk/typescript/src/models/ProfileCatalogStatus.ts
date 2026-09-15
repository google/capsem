// Generated from Capsem OpenAPI. Do not edit.

import type { AssetManifestStatus } from "./AssetManifestStatus.js";
import type { ProfileCatalogSource } from "./ProfileCatalogSource.js";
import type { ProfileReadiness } from "./ProfileReadiness.js";

export interface ProfileCatalogStatus {
  "asset_manifest"?: null | AssetManifestStatus;
  "bytes_done"?: number | null;
  "bytes_total"?: number | null;
  "current_asset"?: string | null;
  "downloaded"?: number | null;
  "profile_count": number;
  "profiles": Array<ProfileReadiness>;
  "ready_count": number;
  "reconcile_error"?: string | null;
  "source": ProfileCatalogSource;
}
