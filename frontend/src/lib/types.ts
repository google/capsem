// TypeScript types mirroring Rust structs for Tauri IPC.

/** The outcome of a domain policy evaluation (serde rename_all = "lowercase"). */
export type Decision = 'allowed' | 'denied' | 'error';

/** A single HTTPS request event from web.db (mirrors telemetry.rs NetEvent). */
export interface NetEvent {
  timestamp: number; // epoch seconds (from SystemTime serialization)
  domain: string;
  port: number;
  decision: Decision;
  bytes_sent: number;
  bytes_received: number;
  duration_ms: number;
  method: string | null;
  path: string | null;
  query: string | null;
  status_code: number | null;
  matched_rule: string | null;
  request_headers: string | null;
  response_headers: string | null;
  request_body_preview: string | null;
  response_body_preview: string | null;
  conn_type: string | null;
  process_name: string | null;
  pid: number | null;
}

/** A tool call emitted by the model. */
export interface ToolCallEntry {
  call_index: number;
  call_id: string;
  tool_name: string;
  arguments: string | null;
}

/** A tool result sent back to the model. */
export interface ToolResponseEntry {
  call_id: string;
  content_preview: string | null;
  is_error: boolean;
}

/** A single model API call record. */
export interface ModelCall {
  timestamp: number;
  provider: string;
  model: string | null;
  process_name: string | null;
  pid: number | null;
  method: string;
  path: string;
  stream: boolean;
  system_prompt_preview: string | null;
  messages_count: number;
  tools_count: number;
  request_bytes: number;
  request_body_preview: string | null;
  message_id: string | null;
  status_code: number | null;
  text_content: string | null;
  thinking_content: string | null;
  stop_reason: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  duration_ms: number;
  response_bytes: number;
  estimated_cost_usd: number;
  trace_id: string | null;
  tool_calls: ToolCallEntry[];
  tool_responses: ToolResponseEntry[];
}

/** ModelCall with row ID from the database. */
export interface ModelCallResponse {
  id: number;
  timestamp: number;
  provider: string;
  model: string | null;
  process_name: string | null;
  pid: number | null;
  method: string;
  path: string;
  stream: boolean;
  system_prompt_preview: string | null;
  messages_count: number;
  tools_count: number;
  request_bytes: number;
  request_body_preview: string | null;
  message_id: string | null;
  status_code: number | null;
  text_content: string | null;
  thinking_content: string | null;
  stop_reason: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  duration_ms: number;
  response_bytes: number;
  estimated_cost_usd: number;
  trace_id: string | null;
  tool_calls: ToolCallEntry[];
  tool_responses: ToolResponseEntry[];
}

/** Summary of a trace (one agent turn), aggregated from grouped model calls. */
export interface TraceSummary {
  trace_id: string;
  started_at: number;
  ended_at: number;
  provider: string;
  model: string | null;
  call_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_duration_ms: number;
  total_estimated_cost_usd: number;
  total_tool_calls: number;
  stop_reason: string | null;
  system_prompt_preview: string | null;
}

/** Full detail for a single trace. */
export interface TraceDetail {
  trace_id: string;
  calls: TraceModelCall[];
}

/** A model call within a trace, with row ID and flattened ModelCall fields. */
export interface TraceModelCall {
  id: number;
  // Flattened from ModelCall via serde(flatten)
  timestamp: number;
  provider: string;
  model: string | null;
  process_name: string | null;
  pid: number | null;
  method: string;
  path: string;
  stream: boolean;
  system_prompt_preview: string | null;
  messages_count: number;
  tools_count: number;
  request_bytes: number;
  request_body_preview: string | null;
  message_id: string | null;
  status_code: number | null;
  text_content: string | null;
  thinking_content: string | null;
  stop_reason: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  duration_ms: number;
  response_bytes: number;
  estimated_cost_usd: number;
  trace_id: string | null;
  tool_calls: ToolCallEntry[];
  tool_responses: ToolResponseEntry[];
}

/** Aggregate session statistics (from SQL). */
export interface SessionStats {
  net_total: number;
  net_allowed: number;
  net_denied: number;
  net_error: number;
  net_bytes_sent: number;
  net_bytes_received: number;
  model_call_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_model_duration_ms: number;
  total_tool_calls: number;
  total_estimated_cost_usd: number;
}

/** Domain request count (from GROUP BY). */
export interface DomainCount {
  domain: string;
  count: number;
  allowed: number;
  denied: number;
}

/** Time bucket for charting. */
export interface TimeBucket {
  bucket_start: string;
  allowed: number;
  denied: number;
}

/** Per-provider token usage. */
export interface ProviderTokenUsage {
  provider: string;
  call_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_duration_ms: number;
  total_estimated_cost_usd: number;
}

/** Tool usage count. */
export interface ToolUsageCount {
  tool_name: string;
  count: number;
}

/** Full session stats response. */
export interface SessionStatsResponse {
  stats: SessionStats;
  top_domains: DomainCount[];
  time_buckets: TimeBucket[];
  provider_usage: ProviderTokenUsage[];
  tool_usage: ToolUsageCount[];
}

/** Response from get_network_policy. */
export interface NetworkPolicyResponse {
  allow: string[];
  block: string[];
  default_action: string;
  corp_managed: boolean;
  conflicts: string[];
}

/** Response from get_guest_config. */
export interface GuestConfigResponse {
  env: Record<string, string>;
}

/** A single transition in the VM state machine history. */
export interface TransitionEntry {
  from: string;
  to: string;
  trigger: string;
  duration_ms: number;
}

/** Response from get_vm_state. */
export interface VmStateResponse {
  state: string;
  elapsed_ms: number;
  history: TransitionEntry[];
}

/** The data type of a setting (serde rename_all = "lowercase"). */
export type SettingType =
  | 'text'
  | 'number'
  | 'password'
  | 'url'
  | 'email'
  | 'apikey'
  | 'bool'
  | 'file';

/** A setting value (serde untagged -- bool | number | string). */
export type SettingValue = boolean | number | string;

/** Where a setting's effective value came from (serde rename_all = "lowercase"). */
export type PolicySource = 'default' | 'user' | 'corp';

/** Per-rule HTTP method permissions. */
export interface HttpMethodPermissions {
  domains: string[];
  path: string | null;
  get: boolean;
  post: boolean;
  put: boolean;
  delete: boolean;
  other: boolean;
}

/** Structured metadata for a setting. */
export interface SettingMetadata {
  domains: string[];
  choices: string[];
  min: number | null;
  max: number | null;
  rules: Record<string, HttpMethodPermissions>;
  guest_path?: string | null;
}

/** A fully resolved setting for UI consumption. */
export interface ResolvedSetting {
  id: string;
  category: string;
  name: string;
  description: string;
  setting_type: SettingType;
  default_value: SettingValue;
  effective_value: SettingValue;
  source: PolicySource;
  modified: string | null;
  corp_locked: boolean;
  enabled_by: string | null;
  enabled: boolean;
  metadata: SettingMetadata;
}

/** Response from get_session_info. */
export interface SessionInfo {
  session_id: string;
  mode: string;
  uptime_ms: number;
  scratch_disk_size_gb: number;
  ram_bytes: number;
  total_requests: number;
  allowed_requests: number;
  denied_requests: number;
  error_requests: number;
  bytes_sent: number;
  bytes_received: number;
  model_call_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_tool_calls: number;
  total_estimated_cost_usd: number;
}

/** A session record from main.db. */
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
}

/** Raw SQL query result (columnar format). */
export interface QueryResult {
  columns: string[];
  rows: unknown[][];
}

/** Sidebar view names. */
export type ViewName = 'terminal' | 'sessions' | 'settings';
