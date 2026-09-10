import {EventEmitter, once} from 'node:events';
import {expect, it} from 'vitest';
import {HttpError, MediaType, Method, Transport} from '../src/transport.js';
import {gateway} from './gateway.js';

it('authenticates and encodes paths, query values, JSON and binary bodies', async () => {
  await gateway((request, response) => response.end(request.body), async (url, received) => {
    const transport = new Transport(`${url}/prefix/`, 'secret');
    const body = JSON.stringify({name: 'café'});
    const data = await transport.request(Method.POST, '/vms/{id}', {
      parameters: {id: 'a/b .. café'}, body,
      query: {skip: undefined, empty: null, zero: 0, yes: true, no: false, layers: ['file', 'exec']},
    });
    expect(new TextDecoder().decode(data)).toBe(body);
    const request = received[0];
    expect(request?.method).toBe('POST');
    expect(request?.url).toBe('/prefix/vms/a%2Fb%20%2E%2E%20caf%C3%A9?zero=0&yes=true&no=false&layers=file%2Cexec');
    expect(request?.headers.authorization).toBe('Bearer secret');
    expect(request?.headers.accept).toBe(MediaType.JSON);
    expect(request?.headers['content-type']).toBe(MediaType.JSON);
    const bytes = new Uint8Array([0, 255]);
    expect(await transport.request(Method.POST, '/copy', {body: bytes, contentType: MediaType.BINARY, accept: MediaType.BINARY})).toEqual(bytes);
    expect(received[1]?.headers['content-type']).toBe(MediaType.BINARY);
    transport.close();
  });
});

it('does not follow redirects or retry mutations', async () => {
  await gateway((_, response) => response.end('unexpected'), async (target, forwarded) => {
    await gateway((_, response) => response.writeHead(302, {location: target}).end('redirected'), async (url, received) => {
      const transport = new Transport(url, 'secret');
      await expect(transport.request(Method.POST, '/mutation')).rejects.toMatchObject({status: 302, body: 'redirected'});
      expect(received).toHaveLength(1);
      expect(forwarded).toHaveLength(0);
      transport.close();
    });
  });
});

it('preserves gateway errors with status and response text', async () => {
  await gateway((_, response) => response.writeHead(409).end('conflict'), async url => {
    const transport = new Transport(url, 'secret');
    const request = transport.request(Method.DELETE, '/vm');
    await expect(request).rejects.toBeInstanceOf(HttpError);
    await expect(request).rejects.toMatchObject({name: 'HttpError', status: 409, body: 'conflict', message: 'HTTP 409: conflict'});
    transport.close();
  });
});

it.each(['caller', 'close', 'timeout'])('aborts an in-flight request through %s', async reason => {
  const events = new EventEmitter();
  await gateway((_, response) => { events.emit('received'); response.on('close', () => events.emit('closed')); }, async (url, received) => {
    const transport = new Transport(url, 'secret', {timeoutMs: reason === 'timeout' ? 100 : 30_000});
    const controller = new AbortController();
    const arrived = once(events, 'received');
    const request = transport.request(Method.GET, '/wait', {signal: controller.signal});
    const rejected = expect(request).rejects.toMatchObject({name: reason === 'timeout' ? 'TimeoutError' : 'AbortError'});
    await arrived;
    if (reason === 'caller') controller.abort();
    if (reason === 'close') transport.close();
    await rejected;
    expect(received).toHaveLength(1);
    transport.close();
    await expect(transport.request(Method.GET, '/')).rejects.toThrow('closed');
  });
});

it.each(['file:///tmp/socket', 'http://user:pass@localhost', 'https://host/?x=1', 'https://host/#fragment', 'no-url'])(
  'rejects unsafe or ambiguous gateway URL %s', url => expect(() => new Transport(url, 'secret')).toThrow(),
);
it.each(['', 'bad\nheader', 'bad\rheader'])('rejects token %j', token => {
  expect(() => new Transport('http://localhost', token)).toThrow();
});
it.each([0, -1, NaN, Infinity, 0.5, 2 ** 32])('rejects timeout %s', timeoutMs => {
  expect(() => new Transport('http://localhost', 'secret', {timeoutMs})).toThrow();
});
it.each(['relative', '/query?x=1', '/fragment#x', '/vms/{id}'])('refuses invalid operation path %s before HTTP', async path => {
  const transport = new Transport('http://127.0.0.1:1', 'secret');
  await expect(transport.request(Method.GET, path)).rejects.toThrow('path');
  transport.close();
});
it.each(['', '.', '..'])('refuses path identifiers that fetch would normalize: %j', async id => {
  const transport = new Transport('http://127.0.0.1:1', 'secret');
  await expect(transport.request(Method.GET, '/vms/{id}', {parameters: {id}})).rejects.toThrow('identifier');
  transport.close();
});
