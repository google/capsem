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
  version?: string;
  missing: string[];
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

export interface ResourceSummary {
  total_ram_mb: number;
  total_cpus: number;
  running_count: number;
  stopped_count: number;
  suspended_count: number;
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
  ram_mb: number;
  cpus: number;
  persistent: boolean;
  env?: Record<string, string>;
  from?: string;
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

// POST /read_file/{id}
export interface ReadFileRequest {
  path: string;
}

export interface ReadFileResponse {
  content: string;
}

// POST /write_file/{id}
export interface WriteFileRequest {
  path: string;
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
