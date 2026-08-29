import { TerminalRateLimiter } from './rate-limiter';

type OutputScheduler = (callback: () => void) => number;
type InputScheduler = (callback: () => void) => void;

export class TerminalOutputCoalescer {
  private queue: Uint8Array[] = [];
  private scheduled = false;

  constructor(
    private readonly write: (bytes: Uint8Array) => void,
    private readonly rateLimiter = new TerminalRateLimiter(),
    private readonly schedule: OutputScheduler = (callback) => requestAnimationFrame(callback),
  ) {}

  push(bytes: Uint8Array): void {
    if (bytes.length === 0 || this.rateLimiter.shouldDrop(bytes.length)) return;
    this.queue.push(bytes);
    if (this.scheduled) return;
    this.scheduled = true;
    this.schedule(() => this.flush());
  }

  flush(): void {
    this.scheduled = false;
    if (this.queue.length === 0) return;
    const chunks = this.queue;
    this.queue = [];
    this.write(concatBytes(chunks));
  }

  reset(): void {
    this.queue = [];
    this.scheduled = false;
    this.rateLimiter.reset();
  }
}

export class TerminalInputCoalescer {
  private queue: Uint8Array[] = [];
  private scheduled = false;

  constructor(
    private readonly send: (bytes: Uint8Array) => void,
    private readonly schedule: InputScheduler = (callback) => {
      if (typeof queueMicrotask === 'function') {
        queueMicrotask(callback);
      } else {
        setTimeout(callback, 0);
      }
    },
  ) {}

  push(bytes: Uint8Array): void {
    if (bytes.length === 0) return;
    this.queue.push(bytes);
    if (this.scheduled) return;
    this.scheduled = true;
    this.schedule(() => this.flush());
  }

  flush(): void {
    this.scheduled = false;
    if (this.queue.length === 0) return;
    const chunks = this.queue;
    this.queue = [];
    this.send(concatBytes(chunks));
  }

  reset(): void {
    this.queue = [];
    this.scheduled = false;
  }
}

function concatBytes(chunks: Uint8Array[]): Uint8Array {
  if (chunks.length === 1) return chunks[0];
  const len = chunks.reduce((acc, chunk) => acc + chunk.length, 0);
  const merged = new Uint8Array(len);
  let offset = 0;
  for (const chunk of chunks) {
    merged.set(chunk, offset);
    offset += chunk.length;
  }
  return merged;
}
