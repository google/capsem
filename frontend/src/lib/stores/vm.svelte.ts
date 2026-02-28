// VM state store using Svelte 5 runes.
import { vmStatus, onVmStateChanged } from '../api';

class VmStore {
  vmState = $state('not created');

  statusColor = $derived(
    this.vmState === 'running'
      ? 'text-info'
      : this.vmState === 'stopped' || this.vmState === 'error'
        ? 'text-secondary'
        : this.vmState === 'not created'
          ? 'text-base-content/30'
          : 'text-warning',
  );

  dotColor = $derived(
    this.vmState === 'running'
      ? 'bg-info'
      : this.vmState === 'stopped' || this.vmState === 'error'
        ? 'bg-secondary'
        : this.vmState === 'not created'
          ? 'bg-base-content/30'
          : 'bg-warning',
  );

  isRunning = $derived(this.vmState === 'running');
  terminalRenderer = $state<'webgl' | 'canvas' | ''>('');

  async init() {
    try {
      this.vmState = await vmStatus();
    } catch {
      this.vmState = 'error';
    }
    onVmStateChanged((state) => {
      this.vmState = state;
    });
  }
}

export const vmStore = new VmStore();
