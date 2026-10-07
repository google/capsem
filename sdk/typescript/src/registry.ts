import type {Registry} from './options.js';

/** Preserve the caller's access intent; never turn a non-object into anonymous access. */
export function registryAccess(registry: Registry | undefined): Registry | undefined {
  if (registry === undefined) return undefined;
  if (registry === null || typeof registry !== 'object' || Array.isArray(registry)) {
    throw new TypeError('Registry must be an access object');
  }
  return {...registry};
}
