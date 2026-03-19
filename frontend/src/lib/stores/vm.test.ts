import { describe, it, expect } from 'vitest';
import { vmStore } from './vm.svelte';

describe('VmStore', () => {
  it('isDownloading is true when vmState is downloading', () => {
    vmStore.vmState = 'downloading';
    expect(vmStore.isDownloading).toBe(true);
  });

  it('isDownloading is false for other states', () => {
    for (const state of ['not created', 'booting', 'running', 'stopped', 'error']) {
      vmStore.vmState = state;
      expect(vmStore.isDownloading).toBe(false);
    }
  });

  it('isRunning is true only when running', () => {
    vmStore.vmState = 'running';
    expect(vmStore.isRunning).toBe(true);
    vmStore.vmState = 'downloading';
    expect(vmStore.isRunning).toBe(false);
  });

  it('statusColor reflects downloading as caution', () => {
    vmStore.vmState = 'downloading';
    expect(vmStore.statusColor).toBe('text-caution');
  });

  it('statusColor reflects running as allowed', () => {
    vmStore.vmState = 'running';
    expect(vmStore.statusColor).toBe('text-allowed');
  });

  it('downloadProgress is null by default', () => {
    expect(vmStore.downloadProgress).toBeNull();
  });
});
