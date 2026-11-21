import { describe, expect, it } from 'vitest';
import { ImageCacheState, type ImageInfo } from '@capsem/sdk';
import { selectedSessionImage, sessionCreateRequest } from '../session-create';

const image: ImageInfo = { name: 'code', description: 'Tools', architectures: ['amd64'], cached: ImageCacheState.UNKNOWN, image: `registry.example/code@sha256:${'a'.repeat(64)}` };

describe('desktop session creation', () => {
  it('resolves catalog choices and refuses a selection removed by a catalog refresh', () => {
    expect(selectedSessionImage([image], 'code')).toBe(image);
    expect(selectedSessionImage([image], '')).toBeNull();
    expect(() => selectedSessionImage([], 'code')).toThrow('no longer available');
    expect(() => selectedSessionImage([{ ...image, image: null }], 'code')).toThrow('incompatible');
  });
  it('uses an advertised immutable pin without guessing runtime or cache readiness', () => {
    expect(sessionCreateRequest(' code-dev ', 2048, 2, image)).toEqual({ name: 'code-dev', persistent: true, ram_mb: 2048, cpus: 2, container: { image: image.image, args: [], env: {}, attach: false } });
  });
  it('keeps ordinary VM creation explicit and refuses incompatible image choices', () => {
    expect(sessionCreateRequest('plain', 12288, 4, null)).toEqual({ name: 'plain', persistent: true, ram_mb: 12288, cpus: 4 });
    expect(() => sessionCreateRequest('other', 2048, 2, { ...image, image: null })).toThrow('incompatible');
    expect(() => sessionCreateRequest('  ', 2048, 2, image)).toThrow('Name is required');
  });
  it('uses the selected saved revision without a competing inline image', () => {
    const environment = { environment_id: 'environment-id', revision_id: 'selected-revision' };
    expect(sessionCreateRequest('saved', 2048, 2, null, environment)).toEqual({
      name: 'saved', persistent: true, ram_mb: 2048, cpus: 2, environment,
    });
    expect(() => sessionCreateRequest('saved', 2048, 2, image, environment)).toThrow('either a saved environment or an image');
  });
});
