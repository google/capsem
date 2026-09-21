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

export type { UpdateStatusResponse, UpdateApplyRequest, UpdateActionResponse, UpdateCommandPlan,
  UpdateTrackStatus, SupplyChainEvidence, SupplyChainManifestEvidence, SupplyChainChannelEvidence,
  SupplyChainReference, UpdateTrackState, UpdateCompatibilityState } from '@capsem/sdk';

export interface UpdateCheckRequest {
  dry_run?: boolean;
}

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
