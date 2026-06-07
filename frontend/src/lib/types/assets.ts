/** Per-asset status in GET /assets/status response. */
export interface AssetEntry {
  name: string;
  path?: string;
  status: 'present' | 'missing' | 'corrupted' | 'downloading';
}

/** Response from GET /assets/status and POST /assets/ensure. */
export interface AssetStatusResponse {
  ready: boolean;
  downloading: boolean;
  assets: AssetEntry[];
  asset_version?: string;
  current_asset?: string;
  bytes_done?: number;
  bytes_total?: number;
  error?: string;
  reconcile_error?: string;
  ensured?: boolean;
  downloaded?: number;
}
