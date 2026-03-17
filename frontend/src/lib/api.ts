// Typed Tauri IPC wrappers with automatic mock fallback for browser dev.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isMock, mockApi } from './mock';
import type {
  ConfigIssue,
  DownloadProgress,
  GuestConfigResponse,
  HostConfig,
  McpPolicyInfo,
  McpServerInfo,
  McpToolInfo,
  NetworkPolicyResponse,
  ResolvedSetting,
  SecurityPreset,
  SessionInfo,
  SettingsNode,
  SettingValue,
  UpdateInfo,
  VmStateResponse,
} from './types';

type UnlistenFn = () => void;

function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(cmd, args);
}

function tauriListen<T>(
  event: string,
  callback: (payload: T) => void,
): Promise<UnlistenFn> {
  return listen<T>(event, (e) => callback(e.payload));
}

// ---------------------------------------------------------------------------
// Invoke wrappers (non-SQL commands only)
// ---------------------------------------------------------------------------

export function vmStatus(): Promise<string> {
  if (isMock) return mockApi.vmStatus();
  return tauriInvoke<string>('vm_status');
}

export function serialInput(input: string): Promise<void> {
  if (isMock) return mockApi.serialInput(input);
  return tauriInvoke('serial_input', { input });
}

export function terminalResize(cols: number, rows: number): Promise<void> {
  if (isMock) return mockApi.terminalResize(cols, rows);
  return tauriInvoke('terminal_resize', { cols, rows });
}

/** Poll for terminal output. Returns bytes as a number array. */
export function terminalPoll(): Promise<number[]> {
  return tauriInvoke<number[]>('terminal_poll');
}

export function getGuestConfig(): Promise<GuestConfigResponse> {
  if (isMock) return mockApi.getGuestConfig();
  return tauriInvoke<GuestConfigResponse>('get_guest_config');
}

export function getNetworkPolicy(): Promise<NetworkPolicyResponse> {
  if (isMock) return mockApi.getNetworkPolicy();
  return tauriInvoke<NetworkPolicyResponse>('get_network_policy');
}

export function setGuestEnv(key: string, value: string): Promise<void> {
  if (isMock) return mockApi.setGuestEnv(key, value);
  return tauriInvoke('set_guest_env', { key, value });
}

export function removeGuestEnv(key: string): Promise<void> {
  if (isMock) return mockApi.removeGuestEnv(key);
  return tauriInvoke('remove_guest_env', { key });
}

export function getSettings(): Promise<ResolvedSetting[]> {
  if (isMock) return mockApi.getSettings();
  return tauriInvoke<ResolvedSetting[]>('get_settings');
}

export function getSettingsTree(): Promise<SettingsNode[]> {
  if (isMock) return mockApi.getSettingsTree();
  return tauriInvoke<SettingsNode[]>('get_settings_tree');
}

export function lintConfig(): Promise<ConfigIssue[]> {
  if (isMock) return mockApi.lintConfig();
  return tauriInvoke<ConfigIssue[]>('lint_config');
}

export function listPresets(): Promise<SecurityPreset[]> {
  if (isMock) return mockApi.listPresets();
  return tauriInvoke<SecurityPreset[]>('list_presets');
}

export function applyPreset(id: string): Promise<string[]> {
  if (isMock) return mockApi.applyPreset(id);
  return tauriInvoke<string[]>('apply_preset', { id });
}

export function updateSetting(id: string, value: SettingValue): Promise<void> {
  if (isMock) return mockApi.updateSetting(id, value);
  return tauriInvoke('update_setting', { id, value });
}

export function detectHostConfig(): Promise<HostConfig> {
  if (isMock) return mockApi.detectHostConfig();
  return tauriInvoke<HostConfig>('detect_host_config');
}

export function getVmState(): Promise<VmStateResponse> {
  if (isMock) return mockApi.getVmState();
  return tauriInvoke<VmStateResponse>('get_vm_state');
}

export function getSessionInfo(): Promise<SessionInfo> {
  if (isMock) return mockApi.getSessionInfo();
  return tauriInvoke<SessionInfo>('get_session_info');
}

// ---------------------------------------------------------------------------
// MCP gateway commands
// ---------------------------------------------------------------------------

export function getMcpServers(): Promise<McpServerInfo[]> {
  if (isMock) return mockApi.getMcpServers();
  return tauriInvoke<McpServerInfo[]>('get_mcp_servers');
}

export function getMcpTools(): Promise<McpToolInfo[]> {
  if (isMock) return mockApi.getMcpTools();
  return tauriInvoke<McpToolInfo[]>('get_mcp_tools');
}

export function getMcpPolicy(): Promise<McpPolicyInfo> {
  if (isMock) return mockApi.getMcpPolicy();
  return tauriInvoke<McpPolicyInfo>('get_mcp_policy');
}

export function setMcpServerEnabled(name: string, enabled: boolean): Promise<void> {
  if (isMock) return mockApi.setMcpServerEnabled(name, enabled);
  return tauriInvoke('set_mcp_server_enabled', { name, enabled });
}

export function addMcpServer(
  name: string,
  url: string,
  headers: Record<string, string>,
  bearerToken: string | null,
): Promise<void> {
  if (isMock) return mockApi.addMcpServer(name, url, headers, bearerToken);
  return tauriInvoke('add_mcp_server', { name, url, headers, bearerToken });
}

export function removeMcpServer(name: string): Promise<void> {
  if (isMock) return mockApi.removeMcpServer(name);
  return tauriInvoke('remove_mcp_server', { name });
}

export function setMcpGlobalPolicy(policy: string): Promise<void> {
  if (isMock) return mockApi.setMcpGlobalPolicy(policy);
  return tauriInvoke('set_mcp_global_policy', { policy });
}

export function setMcpDefaultPermission(permission: string): Promise<void> {
  if (isMock) return mockApi.setMcpDefaultPermission(permission);
  return tauriInvoke('set_mcp_default_permission', { permission });
}

export function setMcpToolPermission(tool: string, permission: string): Promise<void> {
  if (isMock) return mockApi.setMcpToolPermission(tool, permission);
  return tauriInvoke('set_mcp_tool_permission', { tool, permission });
}

export function approveMcpTool(tool: string): Promise<void> {
  if (isMock) return mockApi.approveMcpTool(tool);
  return tauriInvoke('approve_mcp_tool', { tool });
}

export function refreshMcpTools(server?: string): Promise<void> {
  if (isMock) return mockApi.refreshMcpTools(server);
  return tauriInvoke('refresh_mcp_tools', { server: server ?? null });
}

// ---------------------------------------------------------------------------
// App update
// ---------------------------------------------------------------------------

export function checkForAppUpdate(): Promise<UpdateInfo | null> {
  if (isMock) return mockApi.checkForAppUpdate();
  return tauriInvoke<UpdateInfo | null>('check_for_app_update');
}

// ---------------------------------------------------------------------------
// Event listeners
// ---------------------------------------------------------------------------

/** vm-state-changed payload is { state: string, trigger: string }. */
interface VmStateChangedPayload {
  state: string;
  trigger: string;
}

export function onSerialOutput(
  callback: (data: number[]) => void,
): Promise<UnlistenFn> {
  if (isMock) return mockApi.onSerialOutput(callback);
  return tauriListen<number[]>('serial-output', callback);
}

export function onVmStateChanged(
  callback: (state: string) => void,
): Promise<UnlistenFn> {
  if (isMock) return mockApi.onVmStateChanged(callback);
  return tauriListen<VmStateChangedPayload>('vm-state-changed', (payload) =>
    callback(payload.state),
  );
}

export function onTerminalSourceChanged(
  callback: (source: string) => void,
): Promise<UnlistenFn> {
  if (isMock) return mockApi.onTerminalSourceChanged(callback);
  return tauriListen<string>('terminal-source-changed', callback);
}

export function openUrl(url: string): Promise<void> {
  if (isMock) {
    window.open(url, '_blank');
    return Promise.resolve();
  }
  return tauriInvoke('open_url', { url });
}

export function onDownloadProgress(
  callback: (progress: DownloadProgress) => void,
): Promise<UnlistenFn> {
  if (isMock) return mockApi.onDownloadProgress(callback);
  return tauriListen<DownloadProgress>('download-progress', callback);
}
