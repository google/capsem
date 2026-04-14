// Reactive VM store. Polls GET /status at a configurable interval and exposes
// VM list + resource summary. Also provides lifecycle methods (stop, delete, etc.).

import * as api from '../api';
import type { VmSummary, ResourceSummary, ProvisionRequest, ForkRequest, ForkResponse } from '../types/gateway';

class VmStore {
  vms = $state<VmSummary[]>([]);
  resourceSummary = $state<ResourceSummary | null>(null);
  serviceStatus = $state<string>('unknown');
  loading = $state(false);
  error = $state<string | null>(null);

  #interval: ReturnType<typeof setInterval> | null = null;

  async refresh(): Promise<void> {
    try {
      const status = await api.getStatus();
      this.vms = status.vms;
      this.resourceSummary = status.resource_summary;
      this.serviceStatus = status.service;
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : 'Failed to fetch status';
    }
  }

  startPolling(intervalMs = 2000): void {
    if (this.#interval) return;
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
    this.loading = true;
    try {
      await api.stopVm(id);
      await this.refresh();
    } finally {
      this.loading = false;
    }
  }

  async suspend(id: string): Promise<void> {
    this.loading = true;
    try {
      await api.suspendVm(id);
      await this.refresh();
    } finally {
      this.loading = false;
    }
  }

  async delete(id: string): Promise<void> {
    this.loading = true;
    try {
      await api.deleteVm(id);
      await this.refresh();
    } finally {
      this.loading = false;
    }
  }

  async resume(name: string): Promise<void> {
    this.loading = true;
    try {
      await api.resumeVm(name);
      await this.refresh();
    } finally {
      this.loading = false;
    }
  }

  async provision(opts: ProvisionRequest): Promise<{ id: string; name: string }> {
    this.loading = true;
    try {
      const result = await api.provisionVm(opts);
      await this.refresh();
      const vm = this.vms.find(v => v.id === result.id);
      return { id: result.id, name: vm?.name ?? result.id };
    } finally {
      this.loading = false;
    }
  }

  async persist(id: string): Promise<void> {
    this.loading = true;
    try {
      await api.persistVm(id);
      await this.refresh();
    } finally {
      this.loading = false;
    }
  }

  async fork(id: string, opts: ForkRequest): Promise<ForkResponse> {
    this.loading = true;
    try {
      const result = await api.forkVm(id, opts);
      await this.refresh();
      return result;
    } finally {
      this.loading = false;
    }
  }

  destroy(): void {
    this.stopPolling();
  }
}

export const vmStore = new VmStore();
