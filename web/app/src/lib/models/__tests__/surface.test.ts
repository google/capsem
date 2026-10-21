import { describe, expect, it } from 'vitest';
import { ContainerState, ContainerSurfaceKind, type ContainerStatusResponse } from '@capsem/sdk';
import { defaultSessionView, hasOpenableSurface, surfaceEmbedUrl, surfaceLauncherUrl, surfaceMayAppear } from '../surface';

const EXPOSURE = '0199df26-d0f2-74f2-a304-ef67b79d1217';

function status(
  state: ContainerState,
  surface?: ContainerStatusResponse['surface'],
): ContainerStatusResponse {
  return { state, image: 'ghcr.io/google/capsem/claude-desktop:1', surface };
}

const granted = { kind: ContainerSurfaceKind.XPRA, port: 14500, exposure_id: EXPOSURE };
const declared = { kind: ContainerSurfaceKind.XPRA, port: 14500 };

describe('app surface', () => {
  it('defaults declared graphical sessions to UI even before their exposure is ready', () => {
    expect(defaultSessionView(status(ContainerState.STARTING, declared))).toBe('ui');
    expect(defaultSessionView(status(ContainerState.RUNNING, granted))).toBe('ui');
    expect(defaultSessionView(status(ContainerState.RUNNING))).toBe('terminal');
    expect(defaultSessionView(null)).toBe('terminal');
  });

  it('embeds through the authenticated launcher with only the public parent origin in its URL', () => {
    const url=new URL(surfaceEmbedUrl('http://127.0.0.1:19222','vm-1','http://localhost:5173'));
    expect(url.pathname).toBe('/vms/vm-1/surface/');
    expect([...url.searchParams]).toEqual([['parent_origin','http://localhost:5173']]);
  });

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
