import {ExecOutputEncoding, type ExecOutput} from './models/index.js';

/** Decode one stdout or stderr value to its exact bytes. */
export function decodeExecOutput(output: ExecOutput): Uint8Array {
  if (output.encoding === ExecOutputEncoding.UTF8) return new TextEncoder().encode(output.data);
  return Uint8Array.from(globalThis.atob(output.data), character => character.charCodeAt(0));
}
