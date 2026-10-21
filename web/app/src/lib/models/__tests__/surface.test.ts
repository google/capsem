import { afterEach, describe, expect, it, vi } from 'vitest';
import { ContainerState, ContainerSurfaceKind, type ContainerStatusResponse } from '@capsem/sdk';
import { hasOpenableSurface, surfaceLauncherUrl, surfaceMayAppear, waitForSurface } from '../surface';

const EXPOSURE = '0199df26-d0f2-74f2-a304-ef67b79d1217';

function status(
  state: ContainerState,
  surface?: ContainerStatusResponse['surface'],
): ContainerStatusResponse {
  return { state, image: 'ghcr.io/google/capsem/claude-desktop:1', surface };
}

const granted = { kind: ContainerSurfaceKind.XPRA, port: 14500, exposure_id: EXPOSURE };
const declared = { kind: ContainerSurfaceKind.XPRA, port: 14500 };

afterEach(() => vi.useRealTimers());

it('waits for the declared graphical surface before opening it', async () => {
  vi.useFakeTimers();
  const read = vi.fn().mockResolvedValueOnce(status(ContainerState.STARTING, declared))
    .mockResolvedValueOnce(status(ContainerState.RUNNING, declared))
    .mockResolvedValue(status(ContainerState.RUNNING, granted));
  const pending = waitForSurface(read);
  await vi.runAllTimersAsync();
  await expect(pending).resolves.toBeUndefined();
  expect(read).toHaveBeenCalledTimes(3);
});

it('reports failed graphical startup and bounds a surface that never appears', async () => {
  await expect(waitForSurface(async () => ({ ...status(ContainerState.FAILED), error: 'App exited' })))
    .rejects.toThrow('App exited');
  vi.useFakeTimers();
  const pending = waitForSurface(async () => status(ContainerState.RUNNING, declared), 1000);
  const failure = expect(pending).rejects.toThrow('still starting');
  await vi.runAllTimersAsync();
  await failure;
});

describe('app surface', () => {
  it('is openable only once the workload runs and its surface is exposed', () => {
    expect(hasOpenableSurface(status(ContainerState.RUNNING, granted))).toBe(true);
    expect(hasOpenableSurface(status(ContainerState.RUNNING, declared))).toBe(false);
    expect(hasOpenableSurface(status(ContainerState.STARTING, granted))).toBe(false);
    expect(hasOpenableSurface(status(ContainerState.EXITED, granted))).toBe(false);
    expect(hasOpenableSurface(status(ContainerState.RUNNING))).toBe(false);
    expect(hasOpenableSurface(null)).toBe(false);
  });

  it('is asked for again only while one may still appear', () => {
    for (const state of [ContainerState.PULLING, ContainerState.STAGING, ContainerState.STARTING]) {
      expect(surfaceMayAppear(status(state))).toBe(true);
    }
    expect(surfaceMayAppear(status(ContainerState.RUNNING, declared))).toBe(true);
    // A terminal workload, a granted surface, an ended workload or none at all.
    expect(surfaceMayAppear(status(ContainerState.RUNNING))).toBe(false);
    expect(surfaceMayAppear(status(ContainerState.RUNNING, granted))).toBe(false);
    expect(surfaceMayAppear(status(ContainerState.EXITED, declared))).toBe(false);
    expect(surfaceMayAppear(status(ContainerState.FAILED, declared))).toBe(false);
    expect(surfaceMayAppear(null)).toBe(false);
  });

  it('opens through the gateway launcher, with no token in the URL', () => {
    expect(surfaceLauncherUrl('http://127.0.0.1:19222', 'vm-1')).toBe('http://127.0.0.1:19222/vms/vm-1/surface/');
    expect(surfaceLauncherUrl('http://127.0.0.1:19222/', 'a b/c')).toBe(
      'http://127.0.0.1:19222/vms/a%20b%2Fc/surface/',
    );
    expect(surfaceLauncherUrl('http://127.0.0.1:19222', 'vm-1')).not.toMatch(/token|\?/);
  });
});
