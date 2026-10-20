// Native Node bindings for package and SDK acceptance tooling. These never
// enter the browser SDK or shipped MCP package; Rust uses capsem-crypto.
import {createHash, randomUUID} from 'node:crypto';

/** @param {string | Uint8Array} bytes */
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export const uuidV4 = () => randomUUID();
