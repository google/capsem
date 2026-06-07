// Output rate limiter for terminal data.
// Prevents a compromised or runaway VM from freezing the browser with
// unbounded output (e.g., cat /dev/urandom).

const DEFAULT_MAX_BYTES_PER_SECOND = 5_000_000; // 5 MB/s
const DEFAULT_WINDOW_MS = 1000;

export class TerminalRateLimiter {
  private maxBytesPerWindow: number;
  private windowMs: number;
  private bytesThisWindow = 0;
  private windowStart = 0;
  private _throttled = false;

  constructor(
    maxBytesPerSecond = DEFAULT_MAX_BYTES_PER_SECOND,
    windowMs = DEFAULT_WINDOW_MS,
  ) {
    this.maxBytesPerWindow = maxBytesPerSecond * (windowMs / 1000);
    this.windowMs = windowMs;
    this.windowStart = performance.now();
  }

  /** Returns true if the data should be dropped. */
  shouldDrop(bytes: number): boolean {
    const now = performance.now();

    if (now - this.windowStart > this.windowMs) {
      this.bytesThisWindow = 0;
      this.windowStart = now;
      this._throttled = false;
    }

    this.bytesThisWindow += bytes;

    if (this.bytesThisWindow > this.maxBytesPerWindow) {
      this._throttled = true;
      return true;
    }

    return false;
  }

  get throttled(): boolean {
    return this._throttled;
  }

  reset(): void {
    this.bytesThisWindow = 0;
    this.windowStart = performance.now();
    this._throttled = false;
  }
}
