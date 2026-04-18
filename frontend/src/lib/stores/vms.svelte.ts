// Reactive VM store. Polls GET /status at a configurable interval and exposes
// VM list + resource summary. Also provides lifecycle methods (stop, delete, etc.).

import * as api from '../api';
import type { VmSummary, ResourceSummary, AssetHealth, ProvisionRequest, ForkRequest, ForkResponse } from '../types/gateway';

class VmStore {
  vms = $state<VmSummary[]>([]);
  resourceSummary = $state<ResourceSummary | null>(null);
  serviceStatus = $state<string>('unknown');
  assetHealth = $state<AssetHealth | null>(null);
  acting = $state(false);
  polled = $state(false);
  showCreateModal = $state(false);

  get loading(): boolean {
    return !this.polled || this.acting;
  }
  error = $state<string | null>(null);

  #interval: ReturnType<typeof setInterval> | null = null;

  async refresh(): Promise<void> {
    try {
      const status = await api.getStatus();
      const prevCount = this.vms.length;
      const prevService = this.serviceStatus;
      this.vms = status.vms;
      this.resourceSummary = status.resource_summary;
      this.serviceStatus = status.service;
      this.assetHealth = status.assets ?? null;
      this.polled = true;
      this.error = null;
      // Only log state transitions, not every 2s poll.
      if (prevCount !== this.vms.length || prevService !== this.serviceStatus) {
        console.log('[vmStore] state change vms=%d service=%s', this.vms.length, this.serviceStatus);
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : 'Failed to fetch status';
      this.polled = true;
      console.error('[vmStore] refresh FAIL:', this.error);
    }
  }

  startPolling(intervalMs = 2000): void {
    if (this.#interval) return;
    console.log('[vmStore] startPolling(%dms)', intervalMs);
    // Initial fetch
    this.refresh();
    this.#interval = setInterval(() => {
      // Pause when tab is hidden
      if (typeof document !== 'undefined' && document.hidden) return;
      this.refresh();
    }, intervalMs);
  }

  stopPolling(): void {
    if (this.#interval) {
      clearInterval(this.#interval);
      this.#interval = null;
    }
  }

  // -- Lifecycle actions --

  async stop(id: string): Promise<void> {
    console.log('[vmStore] stop(%s)', id);
    this.acting = true;
    try {
      await api.stopVm(id);
      await this.refresh();
    } finally {
      this.acting = false;
    }
  }

  async restart(id: string): Promise<void> {
    console.log('[vmStore] restart(%s)', id);
    this.acting = true;
    try {
      const vm = this.vms.find(v => v.id === id);
      const name = vm?.name;
      await api.stopVm(id);
      if (name) await api.resumeVm(name);
      await this.refresh();
    } finally {
      this.acting = false;
    }
  }

  async suspend(id: string): Promise<void> {
    this.acting = true;
    try {
      await api.suspendVm(id);
      await this.refresh();
    } finally {
      this.acting = false;
    }
  }

  async delete(id: string): Promise<void> {
    console.log('[vmStore] delete(%s)', id);
    this.acting = true;
    try {
      await api.deleteVm(id);
      // Optimistic removal -- don't wait for the service to clean up
      this.vms = this.vms.filter(v => v.id !== id);
      await this.refresh();
    } finally {
      this.acting = false;
    }
  }

  async resume(name: string): Promise<void> {
    this.acting = true;
    try {
      await api.resumeVm(name);
      await this.refresh();
    } finally {
      this.acting = false;
    }
  }

  async provision(opts: ProvisionRequest): Promise<{ id: string; name: string }> {
    console.log('[vmStore] provision(%o)', opts);
    this.acting = true;
    try {
      const result = await api.provisionVm(opts);
      await this.refresh();
      const vm = this.vms.find(v => v.id === result.id);
      return { id: result.id, name: vm?.name ?? result.id };
    } finally {
      this.acting = false;
    }
  }

  async persist(id: string, name: string): Promise<void> {
    this.acting = true;
    try {
      await api.persistVm(id, name);
      await this.refresh();
    } finally {
      this.acting = false;
    }
  }

  async fork(id: string, opts: ForkRequest): Promise<ForkResponse> {
    this.acting = true;
    try {
      const result = await api.forkVm(id, opts);
      await this.refresh();
      return result;
    } finally {
      this.acting = false;
    }
  }

  destroy(): void {
    this.stopPolling();
  }
}

export const vmStore = new VmStore();
