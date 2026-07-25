import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ActiveSessionStatsPoller,
  emptyVmStatsSummary,
} from '../active-session-stats';
import type { VmStatsSummary } from '../types/gateway';

const FIRST: VmStatsSummary = {
  total_input_tokens: 21_845,
  total_thinking_tokens: 0,
  total_output_tokens: 128,
  total_estimated_cost: 0,
  total_tool_calls: 0,
  model_call_count: 1,
};

describe('active session stats poller', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('polls only the selected session and stops cleanly', async () => {
    vi.useFakeTimers();
    const load = vi.fn(async () => FIRST);
    const apply = vi.fn();
    const poller = new ActiveSessionStatsPoller(load, apply, 2_000);

    await poller.setActive('vm-1');
    expect(load).toHaveBeenCalledTimes(1);
    expect(load).toHaveBeenLastCalledWith('vm-1');
    expect(apply).toHaveBeenLastCalledWith(FIRST);

    await vi.advanceTimersByTimeAsync(2_000);
    expect(load).toHaveBeenCalledTimes(2);

    await poller.setActive('vm-2');
    expect(load).toHaveBeenCalledTimes(3);
    expect(load).toHaveBeenLastCalledWith('vm-2');

    poller.destroy();
    await vi.advanceTimersByTimeAsync(4_000);
    expect(load).toHaveBeenCalledTimes(3);
  });

  it('clears the toolbar and ignores a stale response after switching tabs', async () => {
    let releaseFirst: ((value: VmStatsSummary) => void) | undefined;
    const load = vi.fn((id: string) => (
      id === 'vm-1'
        ? new Promise<VmStatsSummary>(resolve => { releaseFirst = resolve; })
        : Promise.resolve({ ...FIRST, model_call_count: 2 })
    ));
    const apply = vi.fn();
    const poller = new ActiveSessionStatsPoller(load, apply, 2_000);

    const firstRequest = poller.setActive('vm-1');
    await poller.setActive('vm-2');
    releaseFirst?.(FIRST);
    await firstRequest;

    expect(apply).toHaveBeenCalledWith(emptyVmStatsSummary());
    expect(apply).toHaveBeenLastCalledWith({ ...FIRST, model_call_count: 2 });
    poller.destroy();
  });

  it('does not overlap slow polls for one session', async () => {
    vi.useFakeTimers();
    let release: ((value: VmStatsSummary) => void) | undefined;
    const load = vi.fn(() => new Promise<VmStatsSummary>(resolve => { release = resolve; }));
    const poller = new ActiveSessionStatsPoller(load, vi.fn(), 2_000);

    const initial = poller.setActive('vm-1');
    await vi.advanceTimersByTimeAsync(6_000);
    expect(load).toHaveBeenCalledTimes(1);

    release?.(FIRST);
    await initial;
    await vi.advanceTimersByTimeAsync(2_000);
    expect(load).toHaveBeenCalledTimes(2);
    poller.destroy();
  });
});
