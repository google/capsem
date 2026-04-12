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

export interface VmSummary {
  id: string;
  name: string | null;
  status: string; // "Running" | "Stopped" | "Suspended" | "Error" | "Booting"
  persistent: boolean;
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
}

// POST /provision, POST /run
export interface ProvisionRequest {
  name?: string;
  ram_mb: number;
  cpus: number;
  persistent: boolean;
  env?: Record<string, string>;
  image?: string;
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

// GET /images
export interface ImageInfo {
  name: string;
  size_bytes?: number;
}

// Error shape used by gateway and service
export interface ErrorResponse {
  error: string;
}
