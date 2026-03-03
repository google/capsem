// VM state store using Svelte 5 runes.
import { vmStatus, onVmStateChanged } from '../api';

class VmStore {
  vmState = $state('not created');

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
  terminalRenderer = $state<'webgl' | 'canvas' | ''>('');

  async init() {
    try {
      this.vmState = (await vmStatus()).toLowerCase();
    } catch {
      this.vmState = 'error';
    }
    onVmStateChanged((state) => {
      this.vmState = state.toLowerCase();
    });
  }
}

export const vmStore = new VmStore();
