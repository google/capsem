// Network events store -- polls net_events + SQL aggregates every 2s.
// All counts/aggregates come from individual SQL queries via queryOne/queryAll.
import { netEvents, queryOne, queryAll } from '../api';
import { NET_STATS_SQL, TOP_DOMAINS_SQL, NET_TIME_BUCKETS_SQL, AVG_LATENCY_SQL, METHOD_DIST_SQL, PROCESS_DIST_SQL, MODEL_STATS_SQL, TOOL_COUNT_SQL } from '../sql';
import type { NetEvent, DomainCount, TimeBucket } from '../types';

interface NetStatsRow {
  net_total: number;
  net_allowed: number;
  net_denied: number;
  net_error: number;
  net_bytes_sent: number;
  net_bytes_received: number;
}

interface AvgLatencyRow {
  avg_latency: number;
}

interface MethodDistRow {
  method: string;
  count: number;
}

interface ProcessDistRow {
  process_name: string;
  count: number;
}

interface ModelStatsRow {
  model_call_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_model_duration_ms: number;
  total_estimated_cost_usd: number;
}

interface ToolCountRow {
  count: number;
}

class NetworkStore {
  events = $state<NetEvent[]>([]);
  netStats = $state<NetStatsRow | null>(null);
  topDomains = $state<DomainCount[]>([]);
  timeBuckets = $state<TimeBucket[]>([]);
  avgLatency = $state(0);
  methodDist = $state<MethodDistRow[]>([]);
  processDist = $state<ProcessDistRow[]>([]);
  modelStats = $state<ModelStatsRow | null>(null);
  toolCount = $state(0);

  // SQL-driven derived counts
  totalCalls = $derived(this.netStats?.net_total ?? 0);
  allowedCount = $derived(this.netStats?.net_allowed ?? 0);
  deniedCount = $derived(this.netStats?.net_denied ?? 0);

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
      const [events, stats, domains, buckets, latency, methods, processes, mStats, tCount] = await Promise.all([
        netEvents(200),
        queryOne<NetStatsRow>(NET_STATS_SQL),
        queryAll<DomainCount>(TOP_DOMAINS_SQL),
        queryAll<TimeBucket>(NET_TIME_BUCKETS_SQL),
        queryOne<AvgLatencyRow>(AVG_LATENCY_SQL),
        queryAll<MethodDistRow>(METHOD_DIST_SQL),
        queryAll<ProcessDistRow>(PROCESS_DIST_SQL),
        queryOne<ModelStatsRow>(MODEL_STATS_SQL),
        queryOne<ToolCountRow>(TOOL_COUNT_SQL),
      ]);
      this.events = events;
      this.netStats = stats;
      this.topDomains = domains;
      this.timeBuckets = buckets;
      this.avgLatency = latency?.avg_latency ?? 0;
      this.methodDist = methods;
      this.processDist = processes;
      this.modelStats = mStats;
      this.toolCount = tCount?.count ?? 0;
    } catch {
      // VM not running or net not initialized -- keep stale data.
    }
  }
}

export const networkStore = new NetworkStore();
