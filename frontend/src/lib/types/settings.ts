// Settings types -- mirrors Rust serde serialization in capsem-core/src/net/policy_config/types.rs.
// Do not modify field names or shapes without matching the backend.

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
  source: PolicySource;
  modified: string | null;
  corp_locked: boolean;
  enabled_by: string | null;
  enabled: boolean;
  metadata: SettingMetadata;
}

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
  source: PolicySource;
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

/** A declarative MCP server node in the settings tree. */
export interface McpServerNode {
  kind: 'mcp_server';
  key: string;
  name: string;
  description?: string | null;
  transport: string;
  command?: string | null;
  url?: string | null;
  args: string[];
  env: Record<string, string>;
  headers: Record<string, string>;
  builtin: boolean;
  enabled: boolean;
  source: PolicySource;
  corp_locked: boolean;
}

/** A settings tree node: group, leaf, action, or MCP server. */
export type SettingsNode = SettingsGroup | SettingsLeaf | SettingsAction | McpServerNode;

/** Unified response from load_settings / save_settings. */
export interface SettingsResponse {
  tree: SettingsNode[];
  issues: ConfigIssue[];
  presets: SecurityPreset[];
}

/** A security preset definition. */
export interface SecurityPreset {
  id: string;
  name: string;
  description: string;
  settings: Record<string, SettingValue>;
  mcp: { default_tool_permission?: string } | null;
}

/** MCP server info from the MCP store. */
export interface McpServerInfo {
  name: string;
  url?: string;
  transport: string;
  enabled: boolean;
  builtin: boolean;
  tool_count: number;
  running: boolean;
}

/** MCP tool info from the MCP store. */
export interface McpToolInfo {
  namespaced_name: string;
  original_name: string;
  description: string;
  server_name: string;
  annotations: {
    title?: string;
    read_only_hint?: boolean;
    destructive_hint?: boolean;
    idempotent_hint?: boolean;
    open_world_hint?: boolean;
  };
  pin_hash: string | null;
  approved: boolean;
  pin_changed: boolean;
}

/** MCP policy info from the MCP store. */
export interface McpPolicyInfo {
  global_policy: string;
  default_tool_permission: string;
  blocked_servers: string[];
  tool_permissions: Record<string, string>;
}

/** Info about an available update. */
export interface UpdateInfo {
  version: string;
  current_version: string;
}
