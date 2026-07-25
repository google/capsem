import type { VmStatsSummary } from './types/gateway';

type StatsLoader = (vmId: string) => Promise<VmStatsSummary>;
type StatsConsumer = (stats: VmStatsSummary) => void;

export function emptyVmStatsSummary(): VmStatsSummary {
  return {
    total_input_tokens: 0,
    total_thinking_tokens: 0,
    total_output_tokens: 0,
    total_estimated_cost: 0,
    total_tool_calls: 0,
    model_call_count: 0,
  };
}

/**
 * Owns the one compact stats poll for the visible session. The generation
 * guard prevents a slow response from a previous tab replacing current data.
 */
export class ActiveSessionStatsPoller {
  #activeId: string | null = null;
  #generation = 0;
  #interval: ReturnType<typeof setInterval> | null = null;
  #inFlight = new Set<string>();

  constructor(
    private readonly load: StatsLoader,
    private readonly consume: StatsConsumer,
    private readonly intervalMs = 2_000,
  ) {}

  async setActive(vmId: string | null): Promise<void> {
    if (vmId === this.#activeId) return;
    this.#stopInterval();
    this.#activeId = vmId;
    const generation = ++this.#generation;
    this.consume(emptyVmStatsSummary());
    if (!vmId) return;

    const initial = this.#refresh(vmId, generation);
    this.#interval = setInterval(() => {
      void this.#refresh(vmId, generation);
    }, this.intervalMs);
    await initial;
  }

  destroy(): void {
    this.#stopInterval();
    this.#activeId = null;
    this.#generation++;
  }

  async #refresh(vmId: string, generation: number): Promise<void> {
    if (this.#inFlight.has(vmId)) return;
    this.#inFlight.add(vmId);
    try {
      const stats = await this.load(vmId);
      if (this.#activeId === vmId && this.#generation === generation) {
        this.consume(stats);
      }
    } catch {
      // Preserve the last good sample across a transient gateway failure.
    } finally {
      this.#inFlight.delete(vmId);
    }
  }

  #stopInterval(): void {
    if (this.#interval) {
      clearInterval(this.#interval);
      this.#interval = null;
    }
  }
}
