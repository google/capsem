// A renderer of session-owned state. It never interprets workload history or
// guesses terminal dimensions. State and rendering share one ordered queue.
import type { TerminalState } from '@capsem/sdk';
import { MAX_STREAM_FRAME_BYTES } from './stream-protocol';

interface Renderer {
  resize(cols: number, rows: number): void;
  reset(): void;
  write(bytes: Uint8Array, complete: () => void): void;
}

type Update = { state: TerminalState } | { bytes: Uint8Array };

export class SessionProjection {
  private queue: Update[] = [];
  private pendingBytes = 0;
  private busy = false;
  private revision = -1;
  private accepting = true;

  constructor(private renderer: Renderer, private resync: () => void) {}

  reconnect(): void {
    this.queue = [];
    this.pendingBytes = 0;
    this.revision = -1;
    this.accepting = true;
  }

  state(state: TerminalState): void {
    if (!this.accepting) return;
    if (state.revision < this.revision || (this.revision < 0 && !state.reset)) {
      this.fail();
      return;
    }
    this.revision = state.revision;
    this.queue.push({ state });
    this.checkAndDrain();
  }

  output(bytes: Uint8Array): void {
    if (!this.accepting || this.revision < 0) return;
    this.queue.push({ bytes });
    this.pendingBytes += bytes.length;
    this.checkAndDrain();
  }

  private fail(): void {
    this.accepting = false;
    this.queue = [];
    this.pendingBytes = 0;
    this.resync();
  }

  private checkAndDrain(): void {
    // A slow renderer reconnects to current state rather than dropping bytes
    // from a cursor-sensitive display or accumulating an unbounded history.
    if (this.pendingBytes > MAX_STREAM_FRAME_BYTES * 32 || this.queue.length > 2048) this.fail();
    if (!this.busy && this.accepting) void this.drain();
  }

  private async drain(): Promise<void> {
    this.busy = true;
    try {
      for (let update; (update = this.queue.shift());) {
        if ('state' in update) {
          const state = update.state;
          if (state.reset) this.renderer.reset();
          this.renderer.resize(state.cols, state.rows);
        } else {
          this.pendingBytes -= update.bytes.length;
          await new Promise<void>(resolve => this.renderer.write(update.bytes, resolve));
        }
      }
    } finally {
      this.busy = false;
    }
  }
}
