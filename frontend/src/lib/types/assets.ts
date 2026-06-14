/** Per-asset status in GET /profiles/{profile_id}/assets/status response. */
export interface AssetEntry {
  name: string;
  kind?: string;
  arch?: string;
  path?: string;
  status: 'present' | 'missing' | 'invalid' | 'corrupted' | 'downloading';
  present?: boolean;
  valid?: boolean;
  expected_hash?: string;
  expected_size?: number;
  actual_hash?: string | null;
  actual_size?: number | null;
}

export interface AssetManifestStatus {
  origin: string;
  path: string;
  origin_path?: string;
  origin_source?: string;
  packaged_at?: string;
  refreshed_at?: string;
  validation_status?: 'valid' | 'missing' | 'invalid';
  validation_error?: string;
  blake3?: string;
  format?: number;
  refresh_policy?: string;
  assets_current?: string;
  binaries_current?: string;
}

/** Response from profile asset status and ensure routes. */
export interface AssetStatusResponse {
  ready: boolean;
  downloading: boolean;
  manifest?: AssetManifestStatus;
  assets: AssetEntry[];
  files?: AssetEntry[];
  invalid_assets?: unknown[];
  invalid_files?: unknown[];
  missing_assets?: unknown[];
  errors?: string[];
  asset_version?: string;
  current_asset?: string;
  bytes_done?: number;
  bytes_total?: number;
  error?: string;
  reconcile_error?: string;
  ensured?: boolean;
  downloaded?: number;
}
