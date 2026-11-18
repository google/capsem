/** Browser timers use a signed 32-bit delay; approved startup can exceed it. */
const TIMER_CHUNK_MS = 2 ** 31 - 1;

export function checkedTimeout(timeout: number): number {
  if (!Number.isSafeInteger(timeout) || timeout <= 0) {
    throw new TypeError('Timeout must be a positive safe integer in milliseconds');
  }
  return timeout;
}

/** One owned timer and cancellation links, all released when the request ends. */
export function requestDeadline(timeoutMs: number, signals: readonly AbortSignal[]): {
  signal: AbortSignal; dispose: () => void;
} {
  checkedTimeout(timeoutMs);
  const controller = new AbortController();
  const deadline = performance.now() + timeoutMs;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const links: {signal: AbortSignal; abort: () => void}[] = [];
  const dispose = (): void => {
    clearTimeout(timer);
    timer = undefined;
    for (const link of links) link.signal.removeEventListener('abort', link.abort);
    links.length = 0;
  };
  const abort = (reason: unknown): void => {controller.abort(reason); dispose();};
  const tick = (): void => {
    const remaining = deadline - performance.now();
    if (remaining <= 0) abort(new DOMException('Request timed out', 'TimeoutError'));
    else timer = setTimeout(tick, Math.min(TIMER_CHUNK_MS, Math.ceil(remaining)));
  };
  for (const signal of signals) {
    if (signal.aborted) {abort(signal.reason); break;}
    const link = {signal, abort: (): void => {abort(signal.reason);}};
    links.push(link);
    signal.addEventListener('abort', link.abort, {once: true});
  }
  if (!controller.signal.aborted) tick();
  return {signal: controller.signal, dispose};
}
