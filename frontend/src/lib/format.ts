// Shared formatting utilities for stats, dashboard, and views.

/** Format milliseconds as human-readable duration (e.g., "500ms", "1.5s"). */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Format bytes as human-readable size (e.g., "512 B", "1.5 KB", "3.2 MB"). */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Format ISO timestamp as locale time string. */
export function formatTime(iso: string): string {
  return new Date(iso).toLocaleTimeString();
}

/** Format seconds as compact uptime (e.g., "30s", "5m", "2h 3m"). */
export function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

/** Format USD cost as "$X.XX". */
export function formatCost(usd: number): string {
  return `$${usd.toFixed(2)}`;
}

/** Format token count as compact number (e.g., "500", "1.2K", "3.5M"). */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}K`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Truncate a string to max characters with ellipsis. */
export function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max) + '...' : s;
}

/** Format ISO timestamp as relative age (e.g., "just now", "5m ago", "2h ago"). */
export function fmtAge(ts: string): string {
  if (!ts) return '';
  const mins = Math.floor((Date.now() - new Date(ts).getTime()) / 60000);
  if (mins <= 0) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ago`;
}
