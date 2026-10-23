import type { BrowserAcknowledgeRequest, BrowserRequest } from '@capsem/sdk';
import { browserUrl } from './browser';

type RequestKey = {vmId: string; id: number; ownerGeneration: string | null};
type Acknowledge = (vmId: string, body: BrowserAcknowledgeRequest) => Promise<unknown>;
const same = (left: RequestKey | null, right: RequestKey | null) => !!left && !!right
  && left.vmId === right.vmId && left.id === right.id && left.ownerGeneration === right.ownerGeneration;
const keyOf = (vmId: string, request: BrowserRequest): RequestKey =>
  ({vmId, id:request.id, ownerGeneration:request.owner_generation ?? null});

/** Opening and acknowledging are separate: retrying metadata must never reopen a URL. */
export class BrowserHandoff {
  #accepted = $state<RequestKey | null>(null);
  #error = $state<RequestKey | null>(null);
  #opening = $state<RequestKey | null>(null);
  #ackFailed = $state<RequestKey | null>(null);
  #confirming = $state<RequestKey | null>(null);
  #attempt = 0;

  pending(vmId: string | undefined, request?: BrowserRequest | null): boolean {
    return !!vmId && !!request && browserUrl(request.url) !== null
      && !same(this.#accepted, keyOf(vmId, request));
  }

  failed(vmId: string | undefined, request?: BrowserRequest | null): boolean {
    return !!vmId && !!request && same(this.#error, keyOf(vmId, request));
  }

  confirmationFailed(vmId: string | undefined, request?: BrowserRequest | null): boolean {
    return !!vmId && !!request && same(this.#ackFailed, keyOf(vmId, request));
  }

  async #confirm(key: RequestKey, acknowledge: Acknowledge): Promise<void> {
    if (same(this.#confirming, key)) return;
    this.#confirming = key;
    let confirmed = false;
    try {
      if (!key.ownerGeneration) return;
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          await acknowledge(key.vmId, {owner_generation:key.ownerGeneration, request_id:key.id});
          confirmed = true;
          break;
        } catch (error) {
          const status = error && typeof error === 'object' && 'status' in error ? error.status : undefined;
          if (typeof status === 'number' && status > 0 && status < 500) break;
        }
      }
    } finally {
      if (same(this.#accepted, key)) this.#ackFailed = confirmed ? null : key;
      if (same(this.#confirming, key)) this.#confirming = null;
    }
  }

  async retryConfirmation(vmId: string | undefined, request: BrowserRequest | null | undefined,
    acknowledge: Acknowledge): Promise<void> {
    if (!vmId || !request) return;
    const key = keyOf(vmId, request);
    if (same(this.#accepted, key)) await this.#confirm(key, acknowledge);
  }

  async open(vmId: string | undefined, request: BrowserRequest | null | undefined,
    opener: (url: string, vmId: string) => Promise<void>, acknowledge: Acknowledge): Promise<void> {
    const url = browserUrl(request?.url);
    if (!vmId || !request || !url || !this.pending(vmId, request)) return;
    const key = keyOf(vmId, request);
    if (same(this.#opening, key)) return;
    const attempt = ++this.#attempt;
    this.#opening = key;
    this.#error = null;
    try {
      await opener(url, vmId);
    } catch {
      if (attempt === this.#attempt) this.#error = key;
      return;
    } finally {
      if (same(this.#opening, key)) this.#opening = null;
    }
    if (attempt === this.#attempt) this.#accepted = key;
    await this.#confirm(key, acknowledge);
  }
}
