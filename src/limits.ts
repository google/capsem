export const LIMITS = {
  requestBytes: 256 * 1024,
  sourceBytes: 64 * 1024,
  objectBytes: 64 * 1024,
  contextBytes: 64 * 1024,
  resultBytes: 128 * 1024,
  compileTimeoutMs: 30_000,
  transpileTimeoutMs: 30_000,
  runTimeoutMs: 5_000,
  maxRuns: 25,
} as const;
