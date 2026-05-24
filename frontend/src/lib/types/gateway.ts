// Gateway response types -- mirrors Rust serde serialization in capsem-gateway/src/status.rs
// and capsem-service/src/api.rs. Do not modify field names without matching the backend.

// GET /
export interface HealthResponse {
  ok: boolean;
  version: string;
  service_socket: string;
}

// GET /token
export interface TokenResponse {
  token: string;
}

export interface AssetHealth {
  ready: boolean;
  state: 'checking' | 'updating' | 'ready' | 'error' | 'unknown';
  version?: string;
  arch?: string;
  profile_id?: string | null;
  profile_revision?: string | null;
  profile_payload_hash?: string | null;
  profile_assets?: ProfileAssetProvenance[];
  missing: string[];
  progress?: AssetProgress;
  error?: string;
  retry_count: number;
  retryable: boolean;
  saved_vm_dependencies?: SavedVmAssetDependency[];
}

export interface ProfileAssetProvenance {
  logical_name: string;
  hash: string;
  source_url: string;
  size: number;
  content_type: string;
}

export interface ProfileAssetLocalStatus {
  name: string;
  path: string;
  status: 'present' | 'missing' | 'downloading' | string;
  source_url: string;
  hash?: string;
  size?: number;
  content_type?: string;
}

export interface ProfileMissingAsset {
  name: string;
  path: string;
  source_url?: string;
}

export interface ProfileAssetStatus {
  state: 'ready' | 'missing' | 'error' | string;
  ready: boolean;
  usable_for_vm: boolean;
  profile_id: string;
  profile_revision?: string | null;
  profile_payload_hash?: string | null;
  asset_version?: string | null;
  arch?: string | null;
  assets: ProfileAssetLocalStatus[];
  missing: string[];
  missing_assets: ProfileMissingAsset[];
  error?: string | null;
}

export interface SavedVmAssetDependency {
  vm: string;
  asset_version: string;
  arch: string;
  missing: string[];
  recovery_hint: string;
}

export interface AssetProgress {
  logical_name: string;
  bytes_done: number;
  bytes_total?: number;
  done: boolean;
}

// GET /status
export interface StatusResponse {
  service: string; // "running" | "unavailable"
  gateway_version: string;
  vm_count: number;
  vms: VmSummary[];
  resource_summary: ResourceSummary | null;
  assets?: AssetHealth;
}

export interface VmSummary {
  id: string;
  name: string | null;
  status: string; // "Running" | "Stopped" | "Suspended" | "Error" | "Booting"
  persistent: boolean;
  profile_id?: string | null;
  profile_revision?: string | null;
  profile_status?: VmProfileStatus | null;
  // Telemetry (present for running VMs, absent for stopped)
  uptime_secs?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_estimated_cost?: number;
  total_tool_calls?: number;
  total_mcp_calls?: number;
  total_requests?: number;
  allowed_requests?: number;
  denied_requests?: number;
  total_file_events?: number;
  model_call_count?: number;
}

export type VmProfileStatus = 'current' | 'needs_update' | 'deprecated' | 'revoked' | 'corrupted' | 'unknown';

export interface ResourceSummary {
  total_ram_mb: number;
  total_cpus: number;
  running_count: number;
  stopped_count: number;
  suspended_count: number;
}

export type ProfileRevisionStatus = 'active' | 'deprecated' | 'revoked';

export interface ProfileCatalogRevision {
  revision: string;
  status: ProfileRevisionStatus;
  min_binary?: string | null;
  profile_hash?: string | null;
  current: boolean;
  installed: boolean;
}

export interface ProfileCatalogProfile {
  profile_id: string;
  current_revision?: string | null;
  installed_revision?: string | null;
  asset_status?: ProfileAssetStatus | null;
  revisions: ProfileCatalogRevision[];
}

export interface ProfileCatalogResponse {
  mode: 'settings_profiles_v2';
  manifest_present: boolean;
  default_profile?: string | null;
  catalog_source?: string | null;
  profiles: ProfileCatalogProfile[];
}

export interface ProfileSummary {
  id: string;
  name: string;
  description?: string;
  best_for?: string;
  ui?: 'coding' | 'everyday' | string;
  revision?: string | null;
  icon_svg?: string | null;
}

export interface ProfileListRecord {
  profile: ProfileSummary;
  source: string;
  path?: string | null;
  locked: boolean;
  asset_status?: ProfileAssetStatus | null;
}

export interface ProfileListResponse {
  mode: 'settings_profiles_v2';
  default_profile?: string | null;
  profiles: ProfileListRecord[];
}

export interface ProfileRevisionsResponse {
  mode: 'settings_profiles_v2';
  profile_id: string;
  current_revision?: string | null;
  installed_revision?: string | null;
  revisions: ProfileCatalogRevision[];
}

// GET /list (proxied to service)
export interface ListResponse {
  sandboxes: SandboxInfo[];
}

export interface SandboxInfo {
  id: string;
  name?: string;
  pid: number;
  status: string;
  persistent: boolean;
  ram_mb?: number;
  cpus?: number;
  version?: string;
  forked_from?: string;
  description?: string;
  // Telemetry (populated by /info, absent from /list)
  created_at?: string;
  uptime_secs?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_estimated_cost?: number;
  total_tool_calls?: number;
  total_mcp_calls?: number;
  total_requests?: number;
  allowed_requests?: number;
  denied_requests?: number;
  total_file_events?: number;
  model_call_count?: number;
}

// POST /provision, POST /run
export interface ProvisionRequest {
  name?: string;
  ram_mb?: number;
  cpus?: number;
  persistent: boolean;
  env?: Record<string, string>;
  from?: string;
  profile_id?: string;
  profile_revision?: string;
}

export interface ProvisionResponse {
  id: string;
}

// POST /exec/{id}
export interface ExecRequest {
  command: string;
  timeout_secs?: number;
}

export interface ExecResponse {
  stdout: string;
  stderr: string;
  exit_code: number;
}

// POST /inspect/{id}
export interface InspectRequest {
  sql: string;
}

export interface InspectResponse {
  columns: string[];
  rows: Record<string, string | number | null>[];
}

// Compatibility shape used by api.readFile(), now backed by GET /files/{id}/content.
export interface ReadFileResponse {
  content: string;
}

// POST /fork/{id}
export interface ForkRequest {
  name: string;
  description?: string;
}

export interface ForkResponse {
  name: string;
  size_bytes: number;
}

// Error shape used by gateway and service
export interface ErrorResponse {
  error: string;
}

// GET /stats -- cross-session aggregation from main.db
export interface StatsResponse {
  global: GlobalStats;
  sessions: SessionRecord[];
  top_providers: ProviderSummary[];
  top_tools: ToolSummary[];
  top_mcp_tools: McpToolSummary[];
}

export interface GlobalStats {
  total_sessions: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost: number;
  total_tool_calls: number;
  total_mcp_calls: number;
  total_file_events: number;
  total_requests: number;
  total_allowed: number;
  total_denied: number;
}

export interface SessionRecord {
  id: string;
  mode: string;
  command: string | null;
  status: string;
  created_at: string;
  stopped_at: string | null;
  scratch_disk_size_gb: number;
  ram_bytes: number;
  total_requests: number;
  allowed_requests: number;
  denied_requests: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost: number;
  total_tool_calls: number;
  total_mcp_calls: number;
  total_file_events: number;
  compressed_size_bytes: number | null;
  vacuumed_at: string | null;
  storage_mode: string | null;
  rootfs_hash: string | null;
  rootfs_version: string | null;
  forked_from: string | null;
  persistent: boolean;
}

export interface ProviderSummary {
  provider: string;
  call_count: number;
  input_tokens: number;
  output_tokens: number;
  estimated_cost: number;
  total_duration_ms: number;
}

export interface ToolSummary {
  tool_name: string;
  call_count: number;
  total_bytes: number;
  total_duration_ms: number;
}

export interface McpToolSummary {
  tool_name: string;
  server_name: string;
  call_count: number;
  total_bytes: number;
  total_duration_ms: number;
}

// Runtime enforcement/detection routes.
export type RuntimeRuleKind = 'enforcement' | 'detection';
export type RuntimeRuleScope = 'profile' | 'user' | 'corp' | 'runtime';
export type RuntimeRuleOrigin = 'profile' | 'user' | 'corp' | 'runtime';
export type RuntimeSecurityDecision = 'allow' | 'ask' | 'block' | 'rewrite' | 'throttle';
export type RuntimeSeverity = 'info' | 'low' | 'medium' | 'high' | 'critical';
export type RuntimeConfidence = 'low' | 'medium' | 'high';
export type RuntimeRuleDefinition =
  | {
      kind: 'enforcement';
      decision: RuntimeSecurityDecision;
      reason?: string | null;
    }
  | {
      kind: 'detection';
      sigma_id?: string | null;
      title: string;
      severity: RuntimeSeverity;
      confidence: RuntimeConfidence;
      tags: string[];
    };

export interface RuntimeRuleEntry {
  id: string;
  pack_id?: string | null;
  scope: RuntimeRuleScope;
  origin: RuntimeRuleOrigin;
  definition: RuntimeRuleDefinition;
  enabled: boolean;
  compiled: boolean;
  compile_status: Record<string, unknown>;
  priority: number;
  generation: number;
  condition: string;
  compiled_plan: string;
  match_count: number;
  last_matched_event?: string | null;
  last_matched_unix_ms?: number | null;
}

export interface DebugReport {
  text: string;
  json?: DebugReportJson | null;
}

export interface DebugReportJson {
  schema: string;
  redacted: boolean;
  security_engine: RuntimeSecurityEngineReport;
}

export interface RuntimeSecurityEngineReport {
  present: boolean;
  runtime_rules_store_enabled: boolean;
  runtime_rules_store_path?: string | null;
  enforcement: RuntimeSecurityRegistryReport;
  detection: RuntimeSecurityRegistryReport;
  confirm: RuntimeSecurityConfirmReport;
}

export interface RuntimeSecurityRegistryReport {
  rule_count: number;
  enabled_count: number;
  compiled_count: number;
  error_count: number;
  runtime_scope_count: number;
  profile_scope_count: number;
  scope_counts: Record<string, number>;
  match_count_total: number;
  latest_match_unix_ms?: number | null;
  rules: RuntimeSecurityRuleReport[];
}

export interface RuntimeSecurityRuleReport {
  kind: RuntimeRuleKind;
  id: string;
  pack_id?: string | null;
  scope: RuntimeRuleScope;
  origin: RuntimeRuleOrigin;
  priority: number;
  enabled: boolean;
  compiled: boolean;
  generation: number;
  action?: RuntimeSecurityDecision | null;
  severity?: RuntimeSeverity | null;
  confidence?: RuntimeConfidence | null;
  match_count: number;
  last_matched_event?: string | null;
  last_matched_unix_ms?: number | null;
}

export interface RuntimeSecurityConfirmReport {
  resolver_available: boolean;
  owner?: string | null;
}

export interface RuntimeRuleListResponse {
  kind: RuntimeRuleKind;
  rules: RuntimeRuleEntry[];
}

export interface RuntimeRuleCompileResponse {
  compiled: boolean;
  id: string;
  compiled_plan: string;
}

export interface RuntimeRuleInstallResponse {
  kind: RuntimeRuleKind;
  rule: RuntimeRuleEntry;
}

export interface RuntimeRuleDeleteResponse {
  kind: RuntimeRuleKind;
  id: string;
  removed: boolean;
}

export interface RuntimeEnforcementRuleRequest {
  id: string;
  pack_id?: string | null;
  condition: string;
  priority?: number;
  decision: RuntimeSecurityDecision;
  reason?: string | null;
  enabled?: boolean;
}

export interface RuntimeDetectionRuleRequest {
  id: string;
  pack_id: string;
  sigma_id?: string | null;
  title: string;
  condition: string;
  priority?: number;
  severity: RuntimeSeverity;
  confidence: RuntimeConfidence;
  tags?: string[];
  enabled?: boolean;
}

export interface RuntimeBacktestEvent {
  event_ref?: Record<string, unknown>;
  event: Record<string, unknown>;
  expected?: string;
}

export interface RuntimeEnforcementBacktestRequest {
  rule: RuntimeEnforcementRuleRequest;
  events: RuntimeBacktestEvent[];
  limit?: number;
}

export interface RuntimeDetectionBacktestRequest {
  rule: RuntimeDetectionRuleRequest;
  events: RuntimeBacktestEvent[];
  limit?: number;
}

export interface RuntimeDetectionHuntRequest {
  rules: RuntimeDetectionRuleRequest[];
  events: RuntimeBacktestEvent[];
  limit?: number;
}

export interface RuntimeSessionDetectionHuntRequest {
  rules: RuntimeDetectionRuleRequest[];
  limit?: number;
}

export interface RuntimeMatchedField {
  path: string;
  value: unknown;
}

export interface RuntimeBacktestMatchRow {
  event_ref: Record<string, unknown>;
  rule_id: string;
  pack_id: string;
  evidence_signature: string;
  matched_fields: RuntimeMatchedField[];
  outcome: Record<string, unknown>;
}

export interface RuntimeBacktestResult {
  total_matches: number;
  unique_evidence_matches: number;
  truncated: boolean;
  rows: RuntimeBacktestMatchRow[];
}
