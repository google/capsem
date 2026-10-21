import { describe, expect, it } from 'vitest';
import * as sdk from '@capsem/sdk';
import * as stream from '../terminal/stream-protocol';

describe('shared terminal stream contract', () => {
  it('uses the SDK codec and bounds directly', () => {
    expect(stream.STREAM_SUBPROTOCOL).toBe(sdk.STREAM_SUBPROTOCOL);
    expect(stream.MAX_STREAM_FRAME_BYTES).toBe(sdk.MAX_STREAM_FRAME_BYTES);
    expect(stream.encodeControl).toBe(sdk.encodeControl);
    expect(stream.encodeStdin).toBe(sdk.encodeStdin);
    expect(stream.decodeServerFrame).toBe(sdk.decodeServerFrame);
  });

  it('encodes the VM id and token for a browser upgrade', () => {
    expect(stream.streamUrl('ws://127.0.0.1:19222', 'vm 1', 't/k'))
      .toBe('ws://127.0.0.1:19222/vms/vm%201/stream?token=t%2Fk');
  });
});
