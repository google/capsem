import {expect, it} from 'vitest';
import {Hypervisor, HttpError, ImageCacheState, type Registry} from '../src/index.js';
import {gateway} from './gateway.js';

const pin = `registry.example/code@sha256:${'a'.repeat(64)}`;
const catalog = {catalog: {reference: 'registry.example/catalog:nightly', digest: `sha256:${'c'.repeat(64)}`, channel: 'nightly'}, images: [
  {name: 'code', description: 'Tools', architectures: ['amd64'], cached: 'unknown', image: pin},
  {name: 'other', description: 'Other runtime', architectures: ['arm64'], cached: 'unknown', image: null},
]};
const pull = {image: 'code', resolved: pin, digest: `sha256:${'b'.repeat(64)}`};

it('retains catalog truth and uses registry access for one pull only', async () => {
  await gateway((request, response) => response.end(JSON.stringify(request.url.startsWith('/images?') ? catalog : pull)), async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      const list = await hv.images.list({refresh: true});
      expect(list.catalog?.channel).toBe('nightly');
      expect(list.images[0]?.image).toBe(pin);
      expect(list.images[0]?.cached).toBe(ImageCacheState.UNKNOWN);
      expect(list.images[1]?.image).toBeNull();
      expect((await hv.images.pull('code', {registry: {username: 'robot', password: 'private', ca_pem: 'CA'}})).resolved).toBe(pin);
      await hv.images.pull('code');
      expect(received.map(request => [request.method, request.url])).toEqual([['GET', '/images?refresh=true'], ['POST', '/images/pull'], ['POST', '/images/pull']]);
      for (const request of received) expect(request.headers.authorization).toBe('Bearer gateway-token');
      expect(JSON.parse(received[1]?.body.toString() ?? '')).toEqual({image: 'code', registry: {username: 'robot', password: 'private', ca_pem: 'CA'}});
      expect(JSON.parse(received[2]?.body.toString() ?? '')).toEqual({image: 'code'});
      const images = hv.images;
      hv.close();
      await expect(images.list()).rejects.toThrow('closed');
      await expect(images.pull('code')).rejects.toThrow('closed');
      expect(received).toHaveLength(3);
    } finally { hv.close(); }
  });
});

it('rejects empty images before sending HTTP', async () => {
  await gateway((_request, response) => response.end('{}'), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      for (const image of ['', '  ']) await expect(hv.images.pull(image)).rejects.toThrow('nonempty');
      for (const registry of [null, 'private', 1, []]) {
        await expect(hv.images.pull('code', {registry: registry as unknown as Registry})).rejects.toThrow();
        await expect(hv.create({image: 'code', registry: registry as unknown as Registry})).rejects.toThrow();
      }
      expect(received).toHaveLength(0);
    } finally { hv.close(); }
  });
});

it('rejects incomplete catalogs and unknown cache states', async () => {
  for (const response of [{images: [{name: 'code'}]}, {images: [{...catalog.images[0], cached: 'ready'}]}]) {
    await gateway((_request, reply) => reply.end(JSON.stringify(response)), async (url, received) => {
      const hv = new Hypervisor(url, 'token');
      try {
        await expect(hv.images.list()).rejects.toThrow();
        expect(received).toHaveLength(1);
      } finally { hv.close(); }
    });
  }
});

it('preserves HTTP refusal without replaying prefetch', async () => {
  await gateway((_request, response) => { response.statusCode = 403; response.end('image refused'); }, async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const failure = hv.images.pull('code');
      await expect(failure).rejects.toBeInstanceOf(HttpError);
      await expect(failure).rejects.toMatchObject({status: 403, body: 'image refused'});
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('forwards cancellation without dispatching a cancelled pull', async () => {
  await gateway((_request, response) => response.end(JSON.stringify(pull)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    const controller = new AbortController();
    controller.abort();
    try {
      await expect(hv.images.pull('code', {signal: controller.signal})).rejects.toThrow();
      expect(received).toHaveLength(0);
    } finally { hv.close(); }
  });
});
