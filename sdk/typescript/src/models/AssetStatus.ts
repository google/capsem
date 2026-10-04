// Generated from Capsem OpenAPI. Do not edit.

import type { AssetFileStatus } from "./AssetFileStatus.js";
import type { AssetManifestStatus } from "./AssetManifestStatus.js";

export interface AssetStatus {
  "asset_version"?: string | null;
  "assets": Array<AssetFileStatus>;
  "bytes_done"?: number | null;
  "bytes_total"?: number | null;
  "current_arch": string;
  "current_asset"?: string | null;
  "downloaded"?: number | null;
  "downloading": boolean;
  "errors": Array<string>;
  "manifest": AssetManifestStatus;
  "ready": boolean;
  "reconcile_error"?: string | null;
  "started"?: boolean | null;
}
