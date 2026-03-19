import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import DownloadProgress from './DownloadProgress.svelte';
import { vmStore } from '../stores/vm.svelte';

afterEach(() => {
  cleanup();
  vmStore.downloadProgress = null;
});

describe('DownloadProgress', () => {
  it('renders the download heading', () => {
    render(DownloadProgress);
    expect(screen.getByText('Downloading VM image')).toBeTruthy();
  });

  it('shows 0% with no progress data', () => {
    vmStore.downloadProgress = null;
    render(DownloadProgress);
    expect(screen.getByText('0%')).toBeTruthy();
    expect(screen.getByText('0 B / ...')).toBeTruthy();
  });

  it('shows percentage from progress data', () => {
    vmStore.downloadProgress = {
      asset: 'rootfs',
      bytes_downloaded: 50 * 1024 * 1024,
      total_bytes: 100 * 1024 * 1024,
      phase: 'downloading',
    };
    render(DownloadProgress);
    expect(screen.getByText('50%')).toBeTruthy();
    expect(screen.getByText('50.0 MB / 100.0 MB')).toBeTruthy();
  });

  it('shows connecting phase text', () => {
    vmStore.downloadProgress = {
      asset: 'rootfs',
      bytes_downloaded: 0,
      total_bytes: 0,
      phase: 'connecting',
    };
    render(DownloadProgress);
    expect(screen.getByText(/Connecting/)).toBeTruthy();
  });

  it('shows verifying phase text', () => {
    vmStore.downloadProgress = {
      asset: 'rootfs',
      bytes_downloaded: 100 * 1024 * 1024,
      total_bytes: 100 * 1024 * 1024,
      phase: 'verifying',
    };
    render(DownloadProgress);
    expect(screen.getByText(/Verifying integrity/)).toBeTruthy();
  });

  it('progress bar value reflects percentage', () => {
    vmStore.downloadProgress = {
      asset: 'rootfs',
      bytes_downloaded: 75 * 1024 * 1024,
      total_bytes: 100 * 1024 * 1024,
      phase: 'downloading',
    };
    render(DownloadProgress);
    const bar = document.querySelector('progress') as HTMLProgressElement;
    expect(bar).toBeTruthy();
    expect(bar.value).toBe(75);
    expect(bar.max).toBe(100);
  });
});
