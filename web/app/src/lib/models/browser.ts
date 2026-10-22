import type { BrowserRequest } from '@capsem/sdk';

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

/** Consume fresh requests while attached to one session, without replaying its retained URL. */
export class BrowserRequestCursor {
  #vmId: string | undefined;
  #requestId = 0;

  next(vmId: string | undefined, request?: BrowserRequest | null): string | null {
    if (vmId !== this.#vmId) {
      this.#vmId = vmId;
      this.#requestId = request?.id ?? 0;
      return null;
    }
    if (!request) {
      this.#requestId = 0;
      return null;
    }
    if (!vmId || request.id <= this.#requestId) return null;
    this.#requestId = request.id;
    return browserUrl(request.url);
  }
}
