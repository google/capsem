import { describe, expect, it, vi } from 'vitest';
import { TerminalInputCoalescer, TerminalOutputCoalescer } from '../terminal/io-coalescer';
import { TerminalRateLimiter } from '../terminal/rate-limiter';

describe('TerminalOutputCoalescer', () => {
  it('flushes multiple websocket chunks as one terminal write per animation frame', () => {
    const writes: Uint8Array[] = [];
    const scheduled: (() => void)[] = [];
    const coalescer = new TerminalOutputCoalescer(
      (bytes) => writes.push(bytes),
      new TerminalRateLimiter(10_000, 1000),
      (callback) => {
        scheduled.push(callback);
        return scheduled.length;
      },
    );

    coalescer.push(bytes('hel'));
    coalescer.push(bytes('lo'));
    coalescer.push(bytes(' world'));

    expect(writes).toEqual([]);
    expect(scheduled).toHaveLength(1);

    scheduled[0]();

    expect(writes).toHaveLength(1);
    expect(text(writes[0])).toBe('hello world');
  });

  it('drops output beyond the configured terminal budget before scheduling a write', () => {
    const write = vi.fn();
    const scheduled: (() => void)[] = [];
    const coalescer = new TerminalOutputCoalescer(
      write,
      new TerminalRateLimiter(5, 1000),
      (callback) => {
        scheduled.push(callback);
        return scheduled.length;
      },
    );

    coalescer.push(bytes('12345'));
    coalescer.push(bytes('6'));
    scheduled[0]();

    expect(write).toHaveBeenCalledTimes(1);
    expect(text(write.mock.calls[0][0])).toBe('12345');
  });
});

describe('TerminalInputCoalescer', () => {
  it('batches bursty terminal input into one websocket send without frame latency', () => {
    const sends: Uint8Array[] = [];
    const scheduled: (() => void)[] = [];
    const coalescer = new TerminalInputCoalescer(
      (bytes) => sends.push(bytes),
      (callback) => scheduled.push(callback),
    );

    coalescer.push(bytes('a'));
    coalescer.push(bytes('b'));
    coalescer.push(bytes('\r'));

    expect(sends).toEqual([]);
    expect(scheduled).toHaveLength(1);

    scheduled[0]();

    expect(sends).toHaveLength(1);
    expect(text(sends[0])).toBe('ab\r');
  });
});

function bytes(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function text(value: Uint8Array): string {
  return new TextDecoder().decode(value);
}
