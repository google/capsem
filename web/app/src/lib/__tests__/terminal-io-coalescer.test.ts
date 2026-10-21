import { describe, expect, it } from 'vitest';
import { TerminalInputCoalescer } from '../terminal/io-coalescer';

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

  it('does not replay pending input into a replacement connection', () => {
    const sends: Uint8Array[] = [];
    const scheduled: (() => void)[] = [];
    const coalescer = new TerminalInputCoalescer(bytes => sends.push(bytes), callback => scheduled.push(callback));
    coalescer.push(bytes('old'));
    coalescer.reset();
    scheduled[0]();
    expect(sends).toEqual([]);
    coalescer.push(bytes('new'));
    scheduled[1]();
    expect(sends.map(text)).toEqual(['new']);
  });
});

function bytes(value: string): Uint8Array {
  return new TextEncoder().encode(value);
}

function text(value: Uint8Array): string {
  return new TextDecoder().decode(value);
}
