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

// GET /status
export interface StatusResponse {
  service: string; // "running" | "unavailable"
  gateway_version: string;
  vm_count: number;
  vms: VmSummary[];
  resource_summary: ResourceSummary | null;
}

// GET /update/status
export interface UpdateStatusResponse {
  checked_at?: number | null;
  channel_url?: string | null;
  channel_hash?: string | null;
  validation_status?: string | null;
  validation_error?: string | null;
  stale: boolean;
  last_error?: string | null;
  binary: UpdateTrackStatus;
  assets: UpdateTrackStatus;
  profiles: UpdateTrackStatus;
  images: UpdateTrackStatus;
  supply_chain?: SupplyChainEvidence;
}

export interface UpdateTrackStatus {
  current?: string | null;
  latest?: string | null;
  blocked_reason?: string | null;
  update_available: boolean;
  state: UpdateTrackState;
  compatibility: UpdateCompatibilityState;
}

export interface SupplyChainEvidence {
  manifest: SupplyChainManifestEvidence;
  channel_index: SupplyChainChannelEvidence;
  host_sbom: SupplyChainReference;
  vm_obom: SupplyChainReference;
  attestations: SupplyChainReference[];
}

export interface SupplyChainManifestEvidence {
  origin?: string | null;
  source?: string | null;
  path: string;
  blake3?: string | null;
}

export interface SupplyChainChannelEvidence {
  url?: string | null;
  blake3?: string | null;
}

export interface SupplyChainReference {
  name: string;
  format?: string | null;
  scope?: string | null;
  generator?: string | null;
  release_artifact?: string | null;
  route?: string | null;
  workflow?: string | null;
}

export type UpdateTrackState =
  | 'current'
  | 'update_available'
  | 'unknown'
  | 'not_published';

export type UpdateCompatibilityState =
  | 'compatible'
  | 'unknown'
  | 'not_applicable';

export interface VmSummary {
  id: string;
  name: string | null;
  status: VmLifecycleState;
  persistent: boolean;
  profile_id: string;
  can_resume: boolean;
  resume_blocked_reason?: string;
  available_actions: VmAction[];
  // Telemetry (present for running sessions, absent for stopped)
  uptime_secs?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_estimated_cost?: number;
  total_tool_calls?: number;
  total_requests?: number;
  allowed_requests?: number;
  denied_requests?: number;
  total_file_events?: number;
  model_call_count?: number;
}

export interface ResourceSummary {
  total_ram_mb: number;
  total_cpus: number;
  running_count: number;
  stopped_count: number;
  suspended_count: number;
}

// GET /vms/list (proxied to service)
export interface ListResponse {
  sandboxes: SandboxInfo[];
}

export interface SandboxInfo {
  id: string;
  name?: string;
  pid: number;
  status: VmLifecycleState;
  persistent: boolean;
  can_resume: boolean;
  resume_blocked_reason?: string;
  available_actions: VmAction[];
  ram_mb?: number;
  cpus?: number;
  version?: string;
  forked_from?: string;
  description?: string;
  // Telemetry (populated by /vms/{id}/info, absent from /vms/list)
  created_at?: string;
  uptime_secs?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_estimated_cost?: number;
  total_tool_calls?: number;
  total_requests?: number;
  allowed_requests?: number;
  denied_requests?: number;
  total_file_events?: number;
  model_call_count?: number;
}

// GET /vms/{id}/status
export interface VmStatusResponse {
  id: string;
  status: VmLifecycleState;
  pid?: number;
  persistent: boolean;
  can_resume: boolean;
  resume_blocked_reason?: string;
  available_actions: VmAction[];
  uptime_secs?: number;
  created_at?: string;
  last_error?: string;
}

export type VmLifecycleState =
  | 'Running'
  | 'Stopped'
  | 'Suspended'
  | 'Defunct'
  | 'Incompatible';

export type VmAction =
  | 'pause'
  | 'stop'
  | 'start'
  | 'resume'
  | 'fork'
  | 'delete';

export interface VmActionContract {
  available_actions: VmAction[];
}

// GET /vms/{id}/save/status, GET /vms/{id}/fork/status
export interface VmOperationStatusResponse {
  vm_id: string;
  operation: string;
  status: string;
  in_progress: boolean;
  message?: string;
}

// POST /vms/create, POST /run
export interface ProvisionRequest {
  profile_id: string;
  name?: string;
  ram_mb?: number;
  cpus?: number;
  persistent: boolean;
  env?: Record<string, string>;
  from?: string;
}

export interface ProvisionResponse {
  id: string;
  name: string;
  profile_id: string;
  status: VmLifecycleState;
  persistent: boolean;
  can_resume: boolean;
  available_actions: VmAction[];
  uds_path?: string;
}

// POST /vms/{id}/exec
export interface ExecRequest {
  command: string;
  timeout_secs?: number;
}

export interface ExecResponse {
  stdout: string;
  stderr: string;
  exit_code: number;
}

// POST /vms/{id}/files/read
export interface ReadFileRequest {
  path: string;
}

export interface ReadFileResponse {
  content: string;
}

// POST /vms/{id}/files/write
export interface WriteFileRequest {
  path: string;
  content: string;
}

// POST /vms/{id}/fork
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
