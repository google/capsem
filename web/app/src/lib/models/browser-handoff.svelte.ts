import type { BrowserRequest } from '@capsem/sdk';
import { browserUrl } from './browser';

type RequestKey = {vmId: string; id: number} | null;

/** A retained request is a fallback action until this client opens it. */
export class BrowserHandoff {
  #accepted = $state<RequestKey>(null);
  #error = $state<RequestKey>(null);
  #attempt = 0;

  pending(vmId: string | undefined, request?: BrowserRequest | null): boolean {
    return !!vmId && browserUrl(request?.url) !== null
      && !(this.#accepted?.vmId === vmId && this.#accepted.id === request?.id);
  }

  failed(vmId: string | undefined, request?: BrowserRequest | null): boolean {
    return this.#error !== null && this.#error.vmId === vmId && this.#error.id === request?.id;
  }

  async open(vmId: string | undefined, request: BrowserRequest | null | undefined,
    opener: (url: string, vmId: string) => Promise<void>): Promise<void> {
    const url = browserUrl(request?.url);
    if (!vmId || !request || !url) return;
    const attempt = ++this.#attempt;
    const key = {vmId, id:request.id};
    this.#error = null;
    try {
      await opener(url, vmId);
      if (attempt === this.#attempt) this.#accepted = key;
    } catch {
      if (attempt === this.#attempt) this.#error = key;
    }
  }
}
