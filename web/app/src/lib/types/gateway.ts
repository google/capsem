// Gateway wire types come from the SDK as their consumers migrate.
import type { HypervisorInfo } from '@capsem/sdk';
export type { SandboxInfo, VmSummary, ResourceSummary, ListResponse,
  VmStatsSummaryResponse as VmStatsSummary } from '@capsem/sdk';
export { VmAction, VmLifecycleState } from '@capsem/sdk';

// Offline is local presentation state, never a gateway response.
export type StatusResponse = Omit<HypervisorInfo, 'service'> & {
  service: HypervisorInfo['service'] | 'offline';
};

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

export interface UpdateCheckRequest {
  dry_run?: boolean;
}

export interface UpdateApplyRequest {
  dry_run?: boolean;
  confirmed?: boolean;
}

export interface UpdateCommandPlan {
  program: string;
  args: string[];
}

export interface UpdateActionResponse {
  status: string;
  command: UpdateCommandPlan;
  exit_code?: number | null;
  stdout?: string | null;
  stderr?: string | null;
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
  sha256?: string | null;
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

export type { ProvisionRequest, ProvisionResponse, ForkRequest, ForkResponse } from "@capsem/sdk";

export type { ExecRequest, ExecResponse } from '@capsem/sdk';

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
