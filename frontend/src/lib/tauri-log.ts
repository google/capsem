// Forwards frontend console output + uncaught errors to the Rust tracing
// pipeline when running inside the Tauri webview. In browser mode, no-ops.
//
// Must be called before any other code that might log, so the patches are
// installed by the time anything writes to console.

import { invoke } from '@tauri-apps/api/core';

let installed = false;

// Detect Tauri v2 webview by presence of the invoke bridge, not the opt-in
// window.isTauri global (which may not be set on all platforms).
function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function initTauriLog(): void {
  if (installed) return;
  installed = true;
  if (!inTauri()) return;

  const send = (level: 'info' | 'warn' | 'error', message: string) => {
    invoke('log_frontend', { level, message }).catch(() => {});
  };

  const origLog = console.log;
  const origWarn = console.warn;
  const origError = console.error;

  console.log = (...args: unknown[]) => {
    origLog.apply(console, args);
    send('info', '[console.log] ' + args.map(fmt).join(' '));
  };
  console.warn = (...args: unknown[]) => {
    origWarn.apply(console, args);
    send('warn', '[console.warn] ' + args.map(fmt).join(' '));
  };
  console.error = (...args: unknown[]) => {
    origError.apply(console, args);
    send('error', '[console.error] ' + args.map(fmt).join(' '));
  };

  window.addEventListener('error', (e) => {
    send('error', `[js-error] ${e.message} at ${e.filename}:${e.lineno}:${e.colno}${e.error?.stack ? ' ' + e.error.stack : ''}`);
  });
  window.addEventListener('unhandledrejection', (e) => {
    const reason = e.reason;
    const msg = reason instanceof Error ? (reason.stack ?? reason.message) : String(reason);
    send('error', '[unhandled-rejection] ' + msg);
  });
}

function fmt(v: unknown): string {
  if (v instanceof Error) return v.stack ?? v.message;
  if (typeof v === 'object') {
    try { return JSON.stringify(v); } catch { return String(v); }
  }
  return String(v);
}
