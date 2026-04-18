// Types for the GUI onboarding wizard.

/** Response from GET /setup/state */
export interface SetupStateResponse {
  schema_version: number;
  completed_steps: string[];
  security_preset: string | null;
  providers_done: boolean;
  repositories_done: boolean;
  service_installed: boolean;
  onboarding_completed: boolean;
  corp_config_source: string | null;
}

/** Response from GET /setup/detect */
export interface DetectedConfigSummary {
  git_name: string | null;
  git_email: string | null;
  ssh_public_key_present: boolean;
  anthropic_api_key_present: boolean;
  google_api_key_present: boolean;
  openai_api_key_present: boolean;
  github_token_present: boolean;
  claude_oauth_present: boolean;
  google_adc_present: boolean;
  settings_written: string[];
}

/** Per-asset status in GET /setup/assets response */
export interface AssetEntry {
  name: string;
  status: 'present' | 'missing' | 'corrupted' | 'downloading';
}

/** Response from GET /setup/assets */
export interface AssetStatusResponse {
  ready: boolean;
  downloading: boolean;
  assets: AssetEntry[];
}
