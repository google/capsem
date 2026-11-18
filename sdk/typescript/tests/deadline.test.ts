import {afterEach, expect, it, vi} from 'vitest';
import {requestDeadline} from '../src/deadline.js';

const CHUNK = 2 ** 31 - 1;
afterEach(() => vi.useRealTimers());

it('waits across timer chunks without early expiration or overflow', () => {
  vi.useFakeTimers({toFake: ['setTimeout', 'clearTimeout', 'performance']});
  const pending = requestDeadline(CHUNK + 600_000, []);
  vi.advanceTimersByTime(CHUNK);
  expect(pending.signal.aborted).toBe(false);
  expect(vi.getTimerCount()).toBe(1);
  vi.advanceTimersByTime(599_999);
  expect(pending.signal.aborted).toBe(false);
  vi.advanceTimersByTime(1);
  expect(pending.signal.aborted).toBe(true);
  expect(pending.signal.reason).toMatchObject({name: 'TimeoutError'});
  expect(vi.getTimerCount()).toBe(0);
});

it.each(['completion', 'caller', 'closed', 'already-cancelled'])('releases timer and cancellation links on %s', outcome => {
  vi.useFakeTimers({toFake: ['setTimeout', 'clearTimeout', 'performance']});
  const caller = new AbortController();
  const closed = new AbortController();
  const reason = new Error('original cancellation');
  if (outcome === 'already-cancelled') caller.abort(reason);
  const unlinkCaller = vi.spyOn(caller.signal, 'removeEventListener');
  const unlinkClosed = vi.spyOn(closed.signal, 'removeEventListener');
  const pending = requestDeadline(CHUNK + 600_000, [closed.signal, caller.signal]);
  if (outcome === 'completion') pending.dispose();
  else if (outcome === 'closed') closed.abort(reason);
  else caller.abort(reason);
  expect(vi.getTimerCount()).toBe(0);
  if (outcome !== 'already-cancelled') expect(unlinkCaller).toHaveBeenCalledOnce();
  expect(unlinkClosed).toHaveBeenCalledOnce();
  expect(pending.signal.aborted).toBe(outcome !== 'completion');
  if (outcome !== 'completion') expect(pending.signal.reason).toBe(reason);
  pending.dispose();
  vi.advanceTimersByTime(CHUNK + 600_000);
  if (outcome === 'completion') expect(pending.signal.aborted).toBe(false);
});

it.each(['success', 'fetch-error', 'body-error', 'http-error'])('transport disposes its owned timer on %s', async outcome => {
  vi.useFakeTimers({toFake: ['setTimeout', 'clearTimeout', 'performance']});
  const {Transport, Method} = await import('../src/transport.js');
  const fetch = vi.fn<typeof globalThis.fetch>();
  if (outcome === 'fetch-error') fetch.mockRejectedValue(new TypeError('offline'));
  else if (outcome === 'body-error') fetch.mockResolvedValue(new Response(new ReadableStream({start(controller) {controller.error(new Error('broken body'));}})));
  else fetch.mockResolvedValue(new Response('done', {status: outcome === 'http-error' ? 409 : 200}));
  vi.stubGlobal('fetch', fetch);
  const transport = new Transport('http://localhost', 'secret', {timeoutMs: CHUNK + 600_000});
  try {
    const request = transport.request(Method.GET, '/wait');
    if (outcome === 'success') await expect(request).resolves.toBeInstanceOf(Uint8Array);
    else await expect(request).rejects.toBeDefined();
    expect(vi.getTimerCount()).toBe(0);
    expect(fetch).toHaveBeenCalledTimes(1);
  } finally {
    transport.close();
    vi.unstubAllGlobals();
  }
});
