// TypeScript types mirroring Rust structs for Tauri IPC.

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

/** The data type of a setting (serde rename_all = "snake_case"). */
export type SettingType =
  | 'text'
  | 'number'
  | 'url'
  | 'email'
  | 'apikey'
  | 'bool'
  | 'file'
  | 'kv_map'
  | 'string_list'
  | 'int_list'
  | 'float_list'
  | 'mcp_tool';

/** A setting value (serde untagged -- bool | number | float | { path, content } | string[] | number[] | string). */
export type SettingValue = boolean | number | string | { path: string; content: string } | string[] | number[];

/** Where a setting's effective value came from (serde rename_all = "lowercase"). */
export type SettingsSource = 'default' | 'user' | 'corp';

export type SettingsChangeValue = SettingValue | null;

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
  widget?: string | null;
  side_effect?: string | null;
  hidden?: boolean;
  builtin?: boolean;
  step?: number | null;
  mask?: boolean;
  validator?: string | null;
  origin?: string | null;
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
  source: SettingsSource;
  modified: string | null;
  corp_locked: boolean;
  enabled_by: string | null;
  enabled: boolean;
  metadata: SettingMetadata;
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
export type ViewName = 'terminal' | 'stats' | 'settings' | 'logs';

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
// MCP endpoint types
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
  has_auth_credential: boolean;
  custom_header_count: number;
  source: string;
  enabled: boolean;
  running: boolean;
  tool_count: number;
  is_stdio: boolean;
}

/** Default MCP permission rule exposed from the profile enforcement contract. */
export interface McpDefaultPermission {
  action: ToolPermission;
  source: string;
  rule_id: string | null;
}

/** Info about a discovered MCP tool. */
export interface McpToolInfo {
  namespaced_name: string;
  original_name: string;
  description: string | null;
  server_name: string;
  annotations: ToolAnnotations | null;
  pin_hash: string | null;
  pin_changed: boolean;
  permission_action: ToolPermission;
  permission_source: string;
}

/** Per-tool permission decision. */
export type ToolPermission = 'allow' | 'ask' | 'block';

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
  enabled: boolean;
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
  source: SettingsSource;
  modified: string | null;
  corp_locked: boolean;
  enabled_by: string | null;
  enabled: boolean;
  metadata: SettingMetadata;
}

/** A grammar-driven action node (button/widget, no stored value). */
export interface SettingsAction {
  kind: 'action';
  key: string;
  name: string;
  description?: string | null;
  action: string;
}

/** A settings tree node: group, leaf, or action. */
export type SettingsNode = SettingsGroup | SettingsLeaf | SettingsAction;

/** Unified response from load_settings / save_settings. */
export interface SettingsResponse {
  tree: SettingsNode[];
  issues: ConfigIssue[];
}

/** A structured log event from the Rust backend. */
export interface LogEntry {
  timestamp: string;
  level: 'ERROR' | 'WARN' | 'INFO' | 'DEBUG';
  target: string;
  message: string;
}

/** Log level filter values. */
export type LogLevel = 'error' | 'warn' | 'info' | 'debug';

/** Info about a session that has a capsem.log file. */
export interface LogSessionInfo {
  session_id: string;
  entry_count: number;
}

// ---------------------------------------------------------------------------
// Stats / view data types (UI-side shapes after mapping DB rows)
// ---------------------------------------------------------------------------

/** Per-model aggregated stats. */
export interface ModelStats {
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  estimatedCostUsd: number;
  callCount: number;
}

/** A tool call entry for the stats view. */
export interface ToolCallStat {
  id: string;
  tool: string;
  server: string;
  args: string;
  result: string;
  durationMs: number;
  timestamp: string;
  isError?: number;
}

/** A network request entry for the stats view. */
export interface NetworkEvent {
  id: string;
  method: string;
  url: string;
  domain: string;
  path: string;
  status: number;
  decision: 'allowed' | 'denied';
  durationMs: number;
  bytesSent: number;
  bytesReceived: number;
  timestamp: string;
  requestHeaders?: string | null;
  responseHeaders?: string | null;
  requestBodyPreview?: string | null;
  responseBodyPreview?: string | null;
  matchedRule?: string | null;
}

/** A file event entry for the stats view. */
export interface FileEvent {
  id: string;
  path: string;
  operation: 'created' | 'modified' | 'deleted';
  sizeBytes: number | null;
  timestamp: string;
}

/** A VM log entry (from gateway /logs endpoint). */
export interface VmLogEntry {
  id: string;
  timestamp: string;
  level: 'info' | 'warn' | 'error';
  source: string;
  message: string;
}

/** A file tree node (from gateway file operations). */
export interface FileNode {
  name: string;
  type: 'file' | 'directory';
  path: string;
  children?: FileNode[];
  content?: string;
  sizeBytes?: number;
}

/** A file entry from the host-side files API (GET /vms/{id}/files/list). */
export interface FileEntry {
  name: string;
  path: string;
  type: 'file' | 'directory';
  size: number;
  mtime: number;
  mime?: string;
  label?: string;
  is_text?: boolean;
  children?: FileEntry[];
}

/** Response from GET /vms/{id}/files/list. */
export interface FileListResponse {
  entries: FileEntry[];
}

/** Response from POST /vms/{id}/files/content (upload). */
export interface FileUploadResponse {
  success: boolean;
  size: number;
}

/** Result from getFileContent(). */
export interface FileContentResult {
  text: string;
  blob: Blob;
  size: number;
}
