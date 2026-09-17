import {ExecOutputEncoding, type ExecOutput} from './models/index.js';

/** Decode one stdout or stderr value to its exact bytes. */
export function decodeExecOutput(output: ExecOutput): Uint8Array {
  if (output.encoding === ExecOutputEncoding.UTF8) return new TextEncoder().encode(output.data);
  return Uint8Array.from(globalThis.atob(output.data), character => character.charCodeAt(0));
}

/** The service's ceiling for one exec or run, which is also its default. */
const EXEC_TIMEOUT_CEILING_SECS = 60 * 60;
/** The gateway's budget for readiness, boot and teardown around a command. */
const GATEWAY_REQUEST_BUDGET_SECS = 120;

/**
 * HTTP deadline for exec or run: the service answers only when the command
 * ends, so wait at least as long as the gateway does for it.
 */
export function commandDeadlineMs(fallbackMs: number, timeoutSecs: number | undefined): number {
  return Math.max(fallbackMs, ((timeoutSecs ?? EXEC_TIMEOUT_CEILING_SECS) + GATEWAY_REQUEST_BUDGET_SECS) * 1000);
}
