// Network events store -- polls net_events + session stats every 2s.
// All counts/aggregates come from SQL (via getSessionStats), not from
// counting the events array in JS.
import { netEvents, getSessionStats } from '../api';
import type { NetEvent, SessionStatsResponse } from '../types';

class NetworkStore {
  events = $state<NetEvent[]>([]);
  stats = $state<SessionStatsResponse | null>(null);

  // SQL-driven derived counts (reads from stats, not from counting events)
  totalCalls = $derived(this.stats?.stats.net_total ?? 0);
  allowedCount = $derived(this.stats?.stats.net_allowed ?? 0);
  deniedCount = $derived(this.stats?.stats.net_denied ?? 0);

  private intervalId: ReturnType<typeof setInterval> | null = null;

  start() {
    this.poll();
    this.intervalId = setInterval(() => this.poll(), 2000);
  }

  stop() {
    if (this.intervalId !== null) {
      clearInterval(this.intervalId);
      this.intervalId = null;
    }
  }

  private async poll() {
    try {
      const [events, stats] = await Promise.all([
        netEvents(200),
        getSessionStats(),
      ]);
      this.events = events;
      this.stats = stats;
    } catch {
      // VM not running or net not initialized -- keep stale data.
    }
  }
}

export const networkStore = new NetworkStore();
