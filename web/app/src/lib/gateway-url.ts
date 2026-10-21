/** The gateway used by both the dashboard and its embedded terminals. */
export function gatewayBaseUrl(): string {
  const fallback = 'http://127.0.0.1:19222';
  if (typeof window === 'undefined') return fallback;
  const { origin, port } = window.location;
  // Astro's usual dev server and the desktop shell use the installed service.
  // A UI served by a loopback gateway keeps that gateway, including its port.
  return port !== '5173' && /^http:\/\/(?:127\.0\.0\.1|localhost)(?::\d+)?$/.test(origin)
    ? origin
    : fallback;
}
