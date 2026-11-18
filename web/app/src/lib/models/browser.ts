/** Shared web-navigation policy for terminal links and guest browser requests. */
export function browserUrl(value: unknown): string | null {
  if (typeof value !== 'string' || value.length > 8192 || /[\u0000-\u001f\u007f-\u009f]/u.test(value)) return null;
  try {
    const url = new URL(value);
    return (url.protocol === 'https:' || url.protocol === 'http:')
      && !!url.hostname && !url.username && !url.password && url.href.length <= 8192
      ? url.href : null;
  } catch { return null; }
}
