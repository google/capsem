import { expect, it } from 'vitest';
import { isSurfaceReconnectMessage, surfaceEmbedUrl } from '../models/surface';

const session = {url: 'http://surface.localhost:19444/_capsem/bootstrap', bootstrap_token: 'one-use'};

it('opens each viewer with its selected profile and only a one-use fragment capability', () => {
  const url = new URL(surfaceEmbedUrl('http://localhost:19222/', 'gui/1', 'http://localhost:5173', session, 'remote'));
  expect(url.pathname).toBe('/vms/gui%2F1/surface/');
  expect([...url.searchParams]).toEqual([['parent_origin', 'http://localhost:5173'], ['display_profile', 'remote']]);
  expect(new URL(surfaceEmbedUrl('http://localhost:19222', 'gui', 'tauri://localhost', session)).searchParams.get('display_profile')).toBe('auto');
});

it('accepts reconnect only from the mounted surface frame', () => {
  const frame = {} as Window;
  expect(isSurfaceReconnectMessage({source: frame, data: {type: 'capsem:surface-reconnect'}}, frame)).toBe(true);
  for (const event of [
    {source: {} as Window, data: {type: 'capsem:surface-reconnect'}},
    {source: frame, data: {type: 'capsem:other'}},
    {source: frame, data: null},
    {source: null, data: {type: 'capsem:surface-reconnect'}},
  ]) expect(isSurfaceReconnectMessage(event, frame)).toBe(false);
  expect(isSurfaceReconnectMessage({source: null, data: {type: 'capsem:surface-reconnect'}}, null)).toBe(false);
});
