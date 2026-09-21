// Generated from Capsem OpenAPI. Do not edit.

import type { ValidationStatus } from "./ValidationStatus.js";

export interface AssetManifestStatus {
  "assets_current"?: string | null;
  "binaries_current"?: string | null;
  "blake3"?: string | null;
  "format"?: number | null;
  "origin": string;
  "origin_path"?: string | null;
  "origin_source"?: string | null;
  "packaged_at"?: string | null;
  "path": string;
  "refresh_policy"?: string | null;
  "refreshed_at"?: string | null;
  "validation_error"?: string | null;
  "validation_status": ValidationStatus;
}
