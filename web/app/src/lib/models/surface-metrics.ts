/** Display telemetry is informational, never an input/credential authority. */
const numericFields = ['fps','frameMs','paintMs','decodeMs','decodeErrors','rttMs','mouseMs','keyMs',
  'quality','depth','sourceWidth','sourceHeight','backingWidth','backingHeight','displayWidth','displayHeight','dpr'] as const;
type NumericField = typeof numericFields[number];
export type SurfaceMetrics = Record<NumericField, number | null> & {
  codec: string | null; color: string | null; clientDpi?: number | null; guestDpi?: number | null;
};

export function parseSurfaceMetrics(value: unknown): SurfaceMetrics | null {
  if (value == null || typeof value !== 'object' || Array.isArray(value)) return null;
  const row = value as Record<string, unknown>;
  for (const field of numericFields) {
    const number = row[field];
    if (number !== null && (typeof number !== 'number' || !Number.isFinite(number) || number < 0 || number > 1e9)) return null;
  }
  for (const field of ['codec', 'color']) {
    if (row[field] !== null && (typeof row[field] !== 'string' || row[field].length > 32)) return null;
  }
  // Older running viewers remain usable while a new gateway is qualified.
  for (const field of ['clientDpi', 'guestDpi']) {
    const number = row[field];
    if (number != null && (typeof number !== 'number' || !Number.isFinite(number) || number < 0 || number > 1e9)) return null;
  }
  return row as SurfaceMetrics;
}

export function metricMs(value: number | null): string {
  return value == null ? '—' : `${value.toFixed(1)} ms`;
}

export function metricSize(width: number | null, height: number | null): string {
  return width == null || height == null ? '—' : `${Math.round(width)} × ${Math.round(height)}`;
}
