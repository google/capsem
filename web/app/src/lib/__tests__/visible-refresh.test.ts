import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { watchVisibleRefresh } from '../models/visible-refresh';

let page: EventTarget & { hidden: boolean };
let browser: EventTarget;
beforeEach(() => {
  vi.useFakeTimers();
  page = Object.assign(new EventTarget(), {hidden: false});
  browser = Object.assign(new EventTarget(), {setInterval, clearInterval});
  vi.stubGlobal('document', page);
  vi.stubGlobal('window', browser);
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('visible page refresh', () => {
  it('refreshes on a visible interval and waking events, but skips hidden views', async () => {
    const refresh = vi.fn(async () => {});
    const stop = watchVisibleRefresh(refresh, 30_000);
    expect(refresh).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(refresh).toHaveBeenCalledTimes(1);
    page.hidden = true;
    await vi.advanceTimersByTimeAsync(60_000);
    browser.dispatchEvent(new Event('focus'));
    expect(refresh).toHaveBeenCalledTimes(1);
    page.hidden = false;
    for (const [target, event] of [[page, 'visibilitychange'], [browser, 'focus'], [browser, 'online']] as const) {
      target.dispatchEvent(new Event(event));
      await vi.advanceTimersByTimeAsync(0);
    }
    expect(refresh).toHaveBeenCalledTimes(4);
    stop();
    browser.dispatchEvent(new Event('focus'));
    page.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(refresh).toHaveBeenCalledTimes(4);
  });

  it('does not overlap a slow read and releases the slot after completion', async () => {
    let complete!: () => void;
    const refresh = vi.fn(() => new Promise<void>(resolve => {complete = resolve;}));
    const stop = watchVisibleRefresh(refresh, 1000);
    browser.dispatchEvent(new Event('focus'));
    await vi.advanceTimersByTimeAsync(3000);
    expect(refresh).toHaveBeenCalledTimes(1);
    complete();
    await vi.advanceTimersByTimeAsync(1000);
    expect(refresh).toHaveBeenCalledTimes(2);
    stop();
    complete();
  });
});
