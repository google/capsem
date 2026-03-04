// Chart color constants -- must match @theme tokens in global.css.
// Using oklch() strings directly (supported in modern browsers as SVG fill).

export const colors = {
  allowed: 'oklch(0.74 0.16 233)',
  denied: 'oklch(0.65 0.15 300)',
  caution: 'oklch(0.82 0.189 84)',

  providerAnthropic: 'oklch(0.75 0.14 60)',
  providerGoogle: 'oklch(0.7 0.15 250)',
  providerOpenai: 'oklch(0.72 0.14 145)',
  providerMistral: 'oklch(0.65 0.2 25)',
  providerFallback: 'oklch(0.6 0.05 250)',

  tokenInput: 'oklch(0.7 0.15 250)',
  tokenOutput: 'oklch(0.72 0.14 145)',

  fileCreated: 'oklch(0.74 0.16 233)',
  fileModified: 'oklch(0.72 0.14 185)',
  fileDeleted: 'oklch(0.65 0.15 300)',
} as const;

const PROVIDER_MAP: Record<string, string> = {
  anthropic: colors.providerAnthropic,
  google: colors.providerGoogle,
  openai: colors.providerOpenai,
  mistral: colors.providerMistral,
};

/** Get provider chart color. */
export function providerColor(provider: string): string {
  return PROVIDER_MAP[provider.toLowerCase()] ?? colors.providerFallback;
}

/** Stable palette for MCP server chart colors. */
const SERVER_PALETTE = [
  colors.providerAnthropic,
  colors.providerGoogle,
  colors.providerOpenai,
  colors.providerMistral,
  colors.allowed,
  colors.denied,
  colors.caution,
];

export function serverColor(_name: string, index: number): string {
  return SERVER_PALETTE[index % SERVER_PALETTE.length];
}

/**
 * Provider hue families -- each provider gets a base hue, and models within
 * that provider get variations in lightness/chroma so they're visually related
 * but distinguishable. Colors are stable once assigned.
 */
const PROVIDER_HUES: Record<string, number> = {
  google: 250,    // blue
  anthropic: 60,  // orange
  openai: 145,    // green
  mistral: 25,    // red-orange
};
const FALLBACK_HUE = 280; // purple

const LIGHTNESS_STEPS = [0.70, 0.60, 0.80, 0.55, 0.75];
const CHROMA_STEPS = [0.15, 0.18, 0.12, 0.20, 0.14];

const providerModelIndex = new Map<string, number>();
const modelColorCache = new Map<string, string>();

/** Get a chart color for a model, grouped by provider hue family. */
export function modelColor(model: string, provider: string): string {
  let c = modelColorCache.get(model);
  if (c) return c;

  const p = provider.toLowerCase();
  const hue = PROVIDER_HUES[p] ?? FALLBACK_HUE;
  const idx = providerModelIndex.get(p) ?? 0;
  providerModelIndex.set(p, idx + 1);

  const l = LIGHTNESS_STEPS[idx % LIGHTNESS_STEPS.length];
  const ch = CHROMA_STEPS[idx % CHROMA_STEPS.length];
  c = `oklch(${l} ${ch} ${hue})`;
  modelColorCache.set(model, c);
  return c;
}
