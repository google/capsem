// Stats store -- polls model stats + tool count for the stats bar,
// and manages stats view state (active tab).
import { queryDb, queryOne } from '../db';
import { MODEL_STATS_SQL, TOOL_COUNT_SQL } from '../sql';
import type { ModelStatsRow, StatsTab } from '../types';

class StatsStore {
  // Stats bar data (polled every 2s).
  modelStats = $state<ModelStatsRow | null>(null);
  toolCount = $state(0);

  // View state.
  activeTab = $state<StatsTab>('ai');

  private interval: ReturnType<typeof setInterval> | null = null;

  /** Format total tokens for display (e.g. "45.2K"). */
  totalTokens = $derived(
    this.modelStats
      ? formatTokens(this.modelStats.total_input_tokens + this.modelStats.total_output_tokens)
      : '--',
  );

  /** Format cost for display (e.g. "$0.42"). */
  totalCost = $derived(
    this.modelStats ? formatCost(this.modelStats.total_cost) : '--',
  );

  setTab(tab: StatsTab) {
    this.activeTab = tab;
  }

  async poll() {
    try {
      const [statsResult, toolResult] = await Promise.all([
        queryDb(MODEL_STATS_SQL),
        queryDb(TOOL_COUNT_SQL),
      ]);
      this.modelStats = queryOne<ModelStatsRow>(statsResult);
      const toolRow = queryOne<{ cnt: number }>(toolResult);
      this.toolCount = toolRow?.cnt ?? 0;
    } catch (e) {
      console.error('Stats poll failed:', e);
    }
  }

  start() {
    this.poll();
    this.interval = setInterval(() => this.poll(), 2000);
  }

  stop() {
    if (this.interval) {
      clearInterval(this.interval);
      this.interval = null;
    }
  }
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
  return String(n);
}

function formatCost(n: number): string {
  if (n < 0.01 && n > 0) return '<$0.01';
  return '$' + n.toFixed(2);
}

export const statsStore = new StatsStore();
