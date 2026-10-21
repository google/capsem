import { expect, it } from 'vitest';
import { isSurfaceReconnectMessage } from '../models/surface';

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
