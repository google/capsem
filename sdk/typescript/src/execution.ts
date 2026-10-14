import {ExecOutputEncoding, type ExecOutput} from './models/index.js';

/** Decode one stdout or stderr value to its exact bytes. */
export function decodeExecOutput(output: ExecOutput): Uint8Array {
  if (output.encoding === ExecOutputEncoding.UTF8) return new TextEncoder().encode(output.data);
  return Uint8Array.from(globalThis.atob(output.data), character => character.charCodeAt(0));
}

/** The service's ceiling for one exec or run, which is also its default. */
export const EXEC_TIMEOUT_CEILING_SECS = 60 * 60;
/** The gateway's budget for readiness, boot and teardown around a command. */
export const GATEWAY_REQUEST_BUDGET_SECS = 120;
/**
 * How long the service waits for a container workload to become ready
 * before it answers a create; the image pull happens inside this window.
 */
export const CREATE_READY_SECS = 110;
/** Largest request body and workspace file the HTTP API accepts, in bytes. */
export const MAX_REQUEST_BODY_BYTES = 10 * 1024 * 1024;

/** Creation answers only after workload readiness, so allow its full window. */
export function createDeadlineMs(fallbackMs: number): number {
  return Math.max(fallbackMs, (CREATE_READY_SECS + GATEWAY_REQUEST_BUDGET_SECS) * 1000);
}

/**
 * HTTP deadline for exec or run: the service answers only when the command
 * ends, so wait at least as long as the gateway does for it.
 */
export function commandDeadlineMs(fallbackMs: number, timeoutSecs: number | undefined): number {
  return Math.max(fallbackMs, ((timeoutSecs ?? EXEC_TIMEOUT_CEILING_SECS) + GATEWAY_REQUEST_BUDGET_SECS) * 1000);
}
