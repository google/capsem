// An image's app surface (an Xpra GUI), as the container status reports it.
//
// The surface opens in the user's browser through the gateway's launcher,
// which authenticates itself and hands the browser to the surface's own
// origin; the UI only decides whether there is one to open and where the
// launcher is.

import { ContainerState, ContainerSurfaceKind, type ContainerStatusResponse } from '@capsem/sdk';

/** A running workload whose Xpra surface the service has exposed. */
export function hasOpenableSurface(status: ContainerStatusResponse | null | undefined): boolean {
  return (
    status?.state === ContainerState.RUNNING &&
    status.surface?.kind === ContainerSurfaceKind.XPRA &&
    Boolean(status.surface.exposure_id)
  );
}

/**
 * Whether asking again may still find an openable surface: the workload is
 * on its way up, or it runs and its declared surface is not exposed yet. A
 * session with no workload, a terminal workload, or one that ended has none.
 */
export function surfaceMayAppear(status: ContainerStatusResponse | null | undefined): boolean {
  switch (status?.state) {
    case undefined:
    case ContainerState.EXITED:
    case ContainerState.FAILED:
      return false;
    case ContainerState.RUNNING:
      return status.surface != null && !status.surface.exposure_id;
    default:
      return true;
  }
}

/** The gateway page that opens VM `vmId`'s surface. It carries no token. */
export function surfaceLauncherUrl(gatewayUrl: string, vmId: string): string {
  return `${gatewayUrl.replace(/\/+$/, '')}/vms/${encodeURIComponent(vmId)}/surface/`;
}
