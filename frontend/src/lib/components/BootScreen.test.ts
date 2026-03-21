import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import BootScreen from './BootScreen.svelte';
import { vmStore } from '../stores/vm.svelte';

afterEach(() => {
  cleanup();
  vmStore.downloadProgress = null;
  vmStore.vmState = 'not created';
});

describe('BootScreen', () => {
  it('renders the app name', () => {
    render(BootScreen);
    expect(screen.getByText('Capsem')).toBeTruthy();
  });

  it('shows re-run wizard button', () => {
    render(BootScreen);
    expect(screen.getByText('Re-run Setup Wizard')).toBeTruthy();
  });

  it('shows disabled button when downloading', () => {
    vmStore.vmState = 'downloading';
    vmStore.downloadProgress = {
      asset: 'rootfs',
      bytes_downloaded: 50 * 1024 * 1024,
      total_bytes: 100 * 1024 * 1024,
      phase: 'downloading',
    };
    render(BootScreen);
    const btn = screen.getByText('Downloading...').closest('button')!;
    expect(btn.disabled).toBe(true);
  });

  it('shows disabled button when booting', () => {
    vmStore.vmState = 'booting';
    render(BootScreen);
    const btn = screen.getByText('Starting...').closest('button')!;
    expect(btn.disabled).toBe(true);
  });

  it('shows enabled Let\'s Go button when running', () => {
    vmStore.vmState = 'running';
    render(BootScreen);
    const btn = screen.getByText("Let's Go").closest('button')!;
    expect(btn.disabled).toBe(false);
  });

  it('progress bar reflects percentage', () => {
    vmStore.vmState = 'downloading';
    vmStore.downloadProgress = {
      asset: 'rootfs',
      bytes_downloaded: 75 * 1024 * 1024,
      total_bytes: 100 * 1024 * 1024,
      phase: 'downloading',
    };
    render(BootScreen);
    const bar = document.querySelector('progress') as HTMLProgressElement;
    expect(bar).toBeTruthy();
    expect(bar.value).toBe(75);
  });
});
