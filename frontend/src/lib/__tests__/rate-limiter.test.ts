import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { TerminalRateLimiter } from '../terminal/rate-limiter.ts';

describe('TerminalRateLimiter', () => {
  let now = 0;

  beforeEach(() => {
    now = 1000;
    vi.spyOn(performance, 'now').mockImplementation(() => now);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('allows data under the limit', () => {
    const rl = new TerminalRateLimiter(1000, 1000); // 1000 bytes/s, 1s window
    expect(rl.shouldDrop(500)).toBe(false);
    expect(rl.throttled).toBe(false);
  });

  it('drops data over the limit', () => {
    const rl = new TerminalRateLimiter(1000, 1000);
    expect(rl.shouldDrop(800)).toBe(false);
    expect(rl.shouldDrop(300)).toBe(true); // 1100 > 1000
    expect(rl.throttled).toBe(true);
  });

  it('resets after window elapses', () => {
    const rl = new TerminalRateLimiter(1000, 1000);
    expect(rl.shouldDrop(900)).toBe(false);

    // Advance past the 1s window
    now = 2001;
    expect(rl.shouldDrop(500)).toBe(false);
    expect(rl.throttled).toBe(false);
  });

  it('accumulates bytes within the same window', () => {
    const rl = new TerminalRateLimiter(1000, 1000);
    expect(rl.shouldDrop(400)).toBe(false);
    expect(rl.shouldDrop(400)).toBe(false);
    expect(rl.shouldDrop(201)).toBe(true); // 1001 > 1000
  });

  it('reset() clears state', () => {
    const rl = new TerminalRateLimiter(1000, 1000);
    rl.shouldDrop(1001);
    expect(rl.throttled).toBe(true);

    rl.reset();
    expect(rl.throttled).toBe(false);
    expect(rl.shouldDrop(500)).toBe(false);
  });

  it('uses default 5MB/s limit', () => {
    const rl = new TerminalRateLimiter();
    // 5MB = 5_000_000 bytes per second
    expect(rl.shouldDrop(4_000_000)).toBe(false);
    expect(rl.shouldDrop(1_000_001)).toBe(true);
  });

  it('scales maxBytesPerWindow with custom windowMs', () => {
    // 1000 bytes/s with 500ms window = 500 bytes per window
    const rl = new TerminalRateLimiter(1000, 500);
    expect(rl.shouldDrop(400)).toBe(false);
    expect(rl.shouldDrop(101)).toBe(true); // 501 > 500
  });
});
