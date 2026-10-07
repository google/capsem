import { describe, expect, it } from 'vitest';
import { ImageCacheState, type ImageInfo } from '@capsem/sdk';
import { sessionCreateRequest } from '../session-create';

const image: ImageInfo = { name: 'code', description: 'Tools', architectures: ['amd64'], cached: ImageCacheState.UNKNOWN, image: `registry.example/code@sha256:${'a'.repeat(64)}` };

describe('desktop session creation', () => {
  it('uses an advertised immutable pin without guessing runtime or cache readiness', () => {
    expect(sessionCreateRequest(' code-dev ', 2048, 2, image)).toEqual({ name: 'code-dev', persistent: true, ram_mb: 2048, cpus: 2, container: { image: image.image, args: [], env: {}, attach: false } });
  });
  it('keeps ordinary VM creation explicit and refuses incompatible image choices', () => {
    expect(sessionCreateRequest('plain', 12288, 4, null)).toEqual({ name: 'plain', persistent: true, ram_mb: 12288, cpus: 4 });
    expect(() => sessionCreateRequest('other', 2048, 2, { ...image, image: null })).toThrow('incompatible');
    expect(() => sessionCreateRequest('  ', 2048, 2, image)).toThrow('Name is required');
  });
});
