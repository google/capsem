export function nowMs(): number {
  return Number(process.hrtime.bigint()) / 1_000_000;
}

export async function measure<T>(fn: () => Promise<T>): Promise<{ value: T; ms: number }> {
  const started = nowMs();
  const value = await fn();
  return { value, ms: nowMs() - started };
}

export function roundMs(value: number): number {
  return Math.round(value * 1000) / 1000;
}
