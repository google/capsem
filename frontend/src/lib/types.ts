// TypeScript types mirroring Rust structs for Tauri IPC.

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

/** A setting value (serde untagged -- bool | number | { path, content } | string). */
export type SettingValue = boolean | number | string | { path: string; content: string };

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
  format?: string;
  docs_url?: string | null;
  prefix?: string | null;
  filetype?: string | null;
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
  total_usage_details: Record<string, number>;
  total_tool_calls: number;
  total_estimated_cost_usd: number;
}

/** Raw SQL query result (columnar format). */
export interface QueryResult {
  columns: string[];
  rows: unknown[][];
}

/** Progress of a VM asset download (rootfs). */
export interface DownloadProgress {
  asset: string;
  bytes_downloaded: number;
  total_bytes: number;
  phase: string;
}

/** Info about an available app update. */
export interface UpdateInfo {
  version: string;
  current_version: string;
}

/** Sidebar view names. */
export type ViewName = 'terminal' | 'stats' | 'settings' | 'wizard';

/** Stats panel tab names. */
export type StatsTab = 'ai' | 'tools' | 'network' | 'files';

/** Aggregated model stats (from stats bar polling). */
export interface ModelStatsRow {
  total_input_tokens: number;
  total_output_tokens: number;
  total_cost: number;
  call_count: number;
}

/** A trace summary row (from TRACES_SQL). */
export interface TraceSummary {
  trace_id: string;
  started_at: string;
  provider: string;
  model: string;
  call_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_duration_ms: number;
  total_cost: number;
  total_tool_calls: number;
  stop_reason: string | null;
}

/** A single model call within a trace. */
export interface TraceModelCall {
  id: number;
  timestamp: string;
  provider: string;
  model: string;
  thinking_content: string | null;
  text_content: string | null;
  input_tokens: number;
  output_tokens: number;
  duration_ms: number;
  estimated_cost_usd: number;
  stop_reason: string | null;
  request_body_preview: string | null;
  system_prompt_preview: string | null;
  messages_count: number;
  tools_count: number;
}

/** A tool call entry (joined from tool_calls table). */
export interface ToolCallEntry {
  id: number;
  model_call_id: number;
  call_index: number;
  call_id: string;
  tool_name: string;
  arguments: string | null;
  origin: string;
}

/** A tool response entry (joined from tool_responses table). */
export interface ToolResponseEntry {
  model_call_id: number;
  call_id: string;
  content_preview: string | null;
  is_error: number;
}

/** A span within the trace viewer (thinking, text, or tool call). */
export type SpanType = 'thinking' | 'text' | 'tool' | 'model_input' | 'net_event' | 'mcp_call' | 'file_event';

/** Detail panel selection. */
export interface DetailSelection {
  type: SpanType;
  data: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// MCP gateway types
// ---------------------------------------------------------------------------

/** MCP tool annotations (per MCP spec 2024-11-05). */
export interface ToolAnnotations {
  title?: string | null;
  read_only_hint: boolean;
  destructive_hint: boolean;
  idempotent_hint: boolean;
  open_world_hint: boolean;
}

/** Info about a configured MCP server. */
export interface McpServerInfo {
  name: string;
  url: string;
  has_bearer_token: boolean;
  custom_header_count: number;
  source: string;
  enabled: boolean;
  running: boolean;
  tool_count: number;
  unsupported_stdio: boolean;
}

/** Info about a discovered MCP tool. */
export interface McpToolInfo {
  namespaced_name: string;
  original_name: string;
  description: string | null;
  server_name: string;
  annotations: ToolAnnotations | null;
  pin_hash: string | null;
  approved: boolean;
  pin_changed: boolean;
}

/** Per-tool permission decision. */
export type ToolPermission = 'allow' | 'warn' | 'block';

/** Info about the MCP policy. */
export interface McpPolicyInfo {
  global_policy: string | null;
  default_tool_permission: string;
  blocked_servers: string[];
  tool_permissions: Record<string, string>;
}

/** Settings sub-section identifier (dynamic, derived from TOML tree). */
export type SettingsSection = string;

/** A config validation issue from config_lint(). */
export interface ConfigIssue {
  id: string;
  severity: 'error' | 'warning';
  message: string;
  docs_url?: string | null;
}

/** A settings tree group node. */
export interface SettingsGroup {
  kind: 'group';
  key: string;
  name: string;
  description?: string | null;
  enabled_by?: string | null;
  collapsed: boolean;
  children: SettingsNode[];
}

/** A settings tree leaf node (resolved setting). */
export interface SettingsLeaf {
  kind: 'leaf';
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

/** A settings tree node: either a group or a leaf. */
export type SettingsNode = SettingsGroup | SettingsLeaf;

/** Host configuration detected from the macOS host. */
export interface HostConfig {
  git_name: string | null;
  git_email: string | null;
  ssh_public_key: string | null;
  anthropic_api_key: string | null;
  google_api_key: string | null;
  openai_api_key: string | null;
  github_token: string | null;
}

/** A security preset definition. */
export interface SecurityPreset {
  id: string;
  name: string;
  description: string;
  settings: Record<string, SettingValue>;
  mcp: { default_tool_permission?: string } | null;
}
