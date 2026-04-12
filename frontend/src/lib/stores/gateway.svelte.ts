// Reactive gateway connection state. Wraps api.ts and exposes connection status
// for components. Initialized once from App.svelte onMount.

import * as api from '../api';

const HEALTH_CHECK_INTERVAL = 10000;
const MAX_BACKOFF = 60000;

class GatewayStore {
  connected = $state(false);
  reachable = $state(false);
  version = $state<string | null>(null);
  error = $state<string | null>(null);

  #healthTimeout: ReturnType<typeof setTimeout> | null = null;
  #failCount = 0;

  async init(): Promise<void> {
    try {
      const result = await api.init();
      this.connected = result.connected;
      this.reachable = result.reachable;
      this.version = result.version;
      this.error = result.connected ? null : result.reachable ? 'No auth' : 'Offline';
      this.#failCount = result.connected ? 0 : 1;
      this.scheduleHealthCheck();
    } catch (e) {
      this.connected = false;
      this.reachable = false;
      this.error = e instanceof Error ? e.message : 'Init failed';
      this.#failCount = 1;
      this.scheduleHealthCheck();
    }
  }

  private scheduleHealthCheck(): void {
    if (this.#healthTimeout) return;
    // Exponential backoff when disconnected, fixed interval when connected
    const delay = this.connected
      ? HEALTH_CHECK_INTERVAL
      : Math.min(HEALTH_CHECK_INTERVAL * Math.pow(2, this.#failCount), MAX_BACKOFF);
    this.#healthTimeout = setTimeout(() => {
      this.#healthTimeout = null;
      this.doHealthCheck();
    }, delay);
  }

  private async doHealthCheck(): Promise<void> {
    // Skip when tab is hidden
    if (typeof document !== 'undefined' && document.hidden) {
      this.scheduleHealthCheck();
      return;
    }

    if (!this.connected) {
      // Try full init (includes token fetch)
      const result = await api.init();
      this.reachable = result.reachable;
      this.version = result.version;
      if (result.connected) {
        this.connected = true;
        this.error = null;
        this.#failCount = 0;
      } else {
        this.error = result.reachable ? 'No auth' : 'Offline';
        this.#failCount = Math.min(this.#failCount + 1, 6);
      }
    } else {
      // Just probe health to detect disconnection
      const ok = await api.healthCheck();
      if (!ok) {
        this.connected = false;
        this.reachable = false;
        this.error = 'Gateway connection lost';
        this.#failCount = 1;
      }
    }

    this.scheduleHealthCheck();
  }

  stopHealthCheck(): void {
    if (this.#healthTimeout) {
      clearTimeout(this.#healthTimeout);
      this.#healthTimeout = null;
    }
  }

  destroy(): void {
    this.stopHealthCheck();
  }
}

export const gatewayStore = new GatewayStore();
