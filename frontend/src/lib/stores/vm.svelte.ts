// VM state store using Svelte 5 runes.
import { vmStatus, onVmStateChanged, onDownloadProgress } from '../api';
import type { DownloadProgress } from '../types';

class VmStore {
  vmState = $state('not created');
  /** Error trigger from backend (e.g. assets_not_found, download_failed). */
  errorTrigger = $state<string | null>(null);
  /** Human-readable error message from backend. */
  errorMessage = $state<string | null>(null);
  downloadProgress = $state<DownloadProgress | null>(null);

  statusColor = $derived(
    this.vmState === 'running'
      ? 'text-allowed'
      : this.vmState === 'stopped' || this.vmState === 'error'
        ? 'text-denied'
        : this.vmState === 'not created'
          ? 'text-base-content/30'
          : 'text-caution',
  );

  dotColor = $derived(
    this.vmState === 'running'
      ? 'bg-allowed'
      : this.vmState === 'stopped' || this.vmState === 'error'
        ? 'bg-denied'
        : this.vmState === 'not created'
          ? 'bg-base-content/30'
          : 'bg-caution',
  );

  isRunning = $derived(this.vmState === 'running');
  isDownloading = $derived(this.vmState === 'downloading');
  isError = $derived(this.vmState === 'error');
  isBooting = $derived(this.vmState === 'booting');
  /** True when the VM is not yet ready -- downloading, booting, or errored. */
  showBootScreen = $derived(this.vmState !== 'running' && this.vmState !== 'not created' && this.vmState !== 'stopped');
  terminalRenderer = $state<'webgl' | 'canvas' | ''>('');

  async init() {
    try {
      this.vmState = (await vmStatus()).toLowerCase();
    } catch {
      this.vmState = 'error';
    }
    onVmStateChanged((payload) => {
      this.vmState = payload.state.toLowerCase();
      if (payload.state.toLowerCase() === 'error') {
        this.errorTrigger = payload.trigger ?? null;
        this.errorMessage = payload.message ?? null;
      } else {
        this.errorTrigger = null;
        this.errorMessage = null;
      }
    });
    onDownloadProgress((progress) => {
      this.downloadProgress = progress;
    });
  }
}

export const vmStore = new VmStore();
