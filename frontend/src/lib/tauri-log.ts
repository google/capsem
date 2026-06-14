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

// T5: developer console handle exposed when `?debug=1` is in the URL.
// `window.__capsemDebug.versions()` reports build timestamp + version.
// `window.__capsemDebug.lastWsEvents` is a small ring of the last 5
// websocket events captured by the api.ts onmessage handler.
// `window.__capsemDebug.snapshot()` returns the same diagnostic truth
// the UI reads from gateway routes: status, profile catalog readiness,
// corp config summary, websocket tail, and frontend log path.
//
// This is intentionally a console-only handle, not a UI panel. The
// visual HUD is punted to the frontend-rebuild sprint.
export interface CapsemDebug {
  versions: () => { build_ts: string; version: string };
  dumpLogs: () => Promise<string>;
  snapshot: () => Promise<unknown>;
  lastWsEvents: unknown[];
}

const WS_RING_MAX = 5;
const wsRing: unknown[] = [];

export function recordWsEvent(ev: unknown): void {
  wsRing.push(ev);
  while (wsRing.length > WS_RING_MAX) wsRing.shift();
}

export function maybeInstallDebugHandle(): void {
  if (typeof window === 'undefined') return;
  const url = typeof window.location !== 'undefined' ? window.location : null;
  if (!url) return;
  const params = new URLSearchParams(url.search);
  if (params.get('debug') !== '1') return;

  // Read build-time constants out of globalThis so a missing Vite-define
  // doesn't throw a ReferenceError. The build pipeline can wire these
  // via vite-define / esbuild --define / equivalent.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  const buildTs: string = g.__BUILD_TS__ ?? 'dev';
  const appVersion: string = g.__APP_VERSION__ ?? 'dev';

  const handle: CapsemDebug = {
    versions: () => ({ build_ts: buildTs, version: appVersion }),
    dumpLogs: async () => {
      try {
        return await invoke<string>('dump_frontend_logs');
      } catch (e) {
        return `error: ${String(e)}`;
      }
    },
    snapshot: async () => {
      const api = await import('./api');
      const [gateway, frontendLog] = await Promise.allSettled([
        api.debugSnapshot(),
        handle.dumpLogs(),
      ]);
      return {
        generated_at: new Date().toISOString(),
        frontend: handle.versions(),
        gateway: gateway.status === 'fulfilled'
          ? gateway.value
          : { error: gateway.reason instanceof Error ? gateway.reason.message : String(gateway.reason) },
        frontend_log: frontendLog.status === 'fulfilled' ? frontendLog.value : String(frontendLog.reason),
        last_ws_events: [...wsRing],
      };
    },
    lastWsEvents: wsRing,
  };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).__capsemDebug = handle;
  // eslint-disable-next-line no-console
  console.info('[capsem-debug] window.__capsemDebug installed (?debug=1)');
}
