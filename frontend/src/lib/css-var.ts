/** Read a CSS custom property from :root for chart.js configs. */
export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** Get provider chart color from CSS custom properties. */
export function providerColor(provider: string): string {
  const key = provider.toLowerCase();
  return cssVar(`--color-provider-${key}`) || cssVar('--color-provider-fallback');
}
