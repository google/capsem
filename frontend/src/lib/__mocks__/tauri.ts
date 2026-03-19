// Stub Tauri IPC for vitest -- prevents real invoke/listen calls.
export function invoke(): Promise<string> {
  return Promise.resolve('');
}
export function listen(): Promise<() => void> {
  return Promise.resolve(() => {});
}
