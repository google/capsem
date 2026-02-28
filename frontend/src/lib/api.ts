// Typed Tauri IPC wrappers with automatic mock fallback for browser dev.
//
// Static imports ensure @tauri-apps/api is bundled into the main chunk.
// Dynamic import() creates code-split chunks that fail to load in Tauri's
// WebView asset protocol.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isMock, mockApi } from './mock';
import type {
  GuestConfigResponse,
  ModelCallResponse,
  NetEvent,
  NetworkPolicyResponse,
  QueryResult,
  ResolvedSetting,
  SessionInfo,
  SessionRecord,
  SessionStatsResponse,
  SettingValue,
  TraceDetail,
  TraceSummary,
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
// Invoke wrappers
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

export function netEvents(limit?: number, search?: string): Promise<NetEvent[]> {
  if (isMock) return mockApi.netEvents(limit, search);
  return tauriInvoke<NetEvent[]>('net_events', { limit: limit ?? 200, search: search ?? null });
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

export function updateSetting(id: string, value: SettingValue): Promise<void> {
  if (isMock) return mockApi.updateSetting(id, value);
  return tauriInvoke('update_setting', { id, value });
}

export function getVmState(): Promise<VmStateResponse> {
  if (isMock) return mockApi.getVmState();
  return tauriInvoke<VmStateResponse>('get_vm_state');
}

export function getSessionInfo(): Promise<SessionInfo> {
  if (isMock) return mockApi.getSessionInfo();
  return tauriInvoke<SessionInfo>('get_session_info');
}

export function getSessionHistory(limit?: number): Promise<SessionRecord[]> {
  if (isMock) return mockApi.getSessionHistory(limit);
  return tauriInvoke<SessionRecord[]>('get_session_history', { limit: limit ?? 50 });
}

export function getSessionStats(): Promise<SessionStatsResponse> {
  if (isMock) return mockApi.getSessionStats();
  return tauriInvoke<SessionStatsResponse>('get_session_stats');
}

export function getModelCalls(limit?: number, search?: string): Promise<ModelCallResponse[]> {
  if (isMock) return mockApi.getModelCalls(limit, search);
  return tauriInvoke<ModelCallResponse[]>('get_model_calls', { limit: limit ?? 50, search: search ?? null });
}

export function getTraces(limit?: number): Promise<TraceSummary[]> {
  if (isMock) return mockApi.getTraces(limit);
  return tauriInvoke<TraceSummary[]>('get_traces', { limit: limit ?? 50 });
}

export function getTraceDetail(traceId: string): Promise<TraceDetail> {
  if (isMock) return mockApi.getTraceDetail(traceId);
  return tauriInvoke<TraceDetail>('get_trace_detail', { traceId });
}

export async function queryDb(sql: string): Promise<QueryResult> {
  if (isMock) return mockApi.queryDb(sql);
  const raw = await tauriInvoke<string>('query_db', { sql });
  return JSON.parse(raw) as QueryResult;
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
