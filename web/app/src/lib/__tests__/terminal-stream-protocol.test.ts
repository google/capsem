import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  STREAM_SUBPROTOCOL,
  MAX_STREAM_FRAME_BYTES,
  decodeServerFrame,
  encodeControl,
  encodeStdin,
  streamUrl,
  type StreamControl,
} from '../terminal/stream-protocol';

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

function frame(channel: number, payload: string | number[]): ArrayBuffer {
  const body = typeof payload === 'string' ? Array.from(new TextEncoder().encode(payload)) : payload;
  return new Uint8Array([channel, ...body]).buffer;
}

describe('capsem.stream.v1 client frames', () => {
  it('prefixes stdin with its channel byte and keeps bytes raw', () => {
    expect(Array.from(encodeStdin(new Uint8Array([0x6c, 0xff])))).toEqual([0, 0x6c, 0xff]);
  });

  it('encodes typed control as channel 3 JSON matching the Rust contract', () => {
    const start = encodeControl({ type: 'start', kind: 'terminal' });
    expect(start[0]).toBe(3);
    expect(text(start.subarray(1))).toBe('{"type":"start","kind":"terminal"}');
    const resize = encodeControl({ type: 'resize', cols: 120, rows: 40 });
    expect(text(resize.subarray(1))).toBe('{"type":"resize","cols":120,"rows":40}');
  });
});

describe('capsem.stream.v1 server frames', () => {
  it('decodes stdout bytes and status JSON', () => {
    expect(decodeServerFrame(frame(1, [0x24, 0x20, 0xff]))).toEqual({
      kind: 'output',
      bytes: new Uint8Array([0x24, 0x20, 0xff]),
    });
    expect(decodeServerFrame(frame(4, '{"type":"error","message":"terminal closed"}'))).toEqual({
      kind: 'status',
      status: { type: 'error', message: 'terminal closed' },
    });
  });

  it('refuses frames a client must never receive', () => {
    expect(decodeServerFrame(new ArrayBuffer(0)).kind).toBe('invalid');
    expect(decodeServerFrame(frame(0, [1])).kind).toBe('invalid');
    expect(decodeServerFrame(frame(3, [1])).kind).toBe('invalid');
    expect(decodeServerFrame(frame(4, '{')).kind).toBe('invalid');
    expect(decodeServerFrame(frame(4, '{"type":"exit"}')).kind).toBe('invalid');
    expect(decodeServerFrame(frame(9, [])).kind).toBe('invalid');
  });
});

describe('stream URL', () => {
  it('names the VM stream route with the token for the browser upgrade', () => {
    expect(STREAM_SUBPROTOCOL).toBe('capsem.stream.v1');
    expect(streamUrl('ws://127.0.0.1:19222', 'vm 1', 't/k')).toBe('ws://127.0.0.1:19222/vms/vm%201/stream?token=t%2Fk');
  });
});

// The service bounds what it sends; a client that trusts an unbounded frame
// would render whatever a non-stock guest chose to emit.
it('refuses a frame past the protocol ceiling', () => {
  const oversized = new Uint8Array(MAX_STREAM_FRAME_BYTES + 1);
  oversized[0] = 1;
  expect(decodeServerFrame(oversized.buffer)).toEqual({
    kind: 'invalid',
    reason: `stream frame of ${MAX_STREAM_FRAME_BYTES + 1} bytes exceeds ${MAX_STREAM_FRAME_BYTES}`,
  });
  const largest = new Uint8Array(MAX_STREAM_FRAME_BYTES);
  largest[0] = 1;
  expect(decodeServerFrame(largest.buffer).kind).toBe('output');
});

// Finding 32: this codec is a TypeScript copy of capsem-api's stream module.
// The fixture is generated from the Rust codec, so any drift -- a renamed
// field, a moved channel, a changed bound -- fails here instead of in a
// browser terminal.
interface GoldenFrames {
  subprotocol: string;
  max_frame_bytes: number;
  client: Array<{ control?: StreamControl; stdin?: number[]; frame: number[] }>;
  server: Array<{ frame: number[]; expect: { kind: string; bytes?: number[]; status?: unknown } }>;
  oversized_frame_bytes: number;
}

const golden: GoldenFrames = JSON.parse(
  readFileSync(new URL('../../../../../sdk/specification/stream-v1.json', import.meta.url), 'utf8'),
);

describe('capsem.stream.v1 against the Rust golden frames', () => {
  it('shares the protocol name and frame bound', () => {
    expect(STREAM_SUBPROTOCOL).toBe(golden.subprotocol);
    expect(MAX_STREAM_FRAME_BYTES).toBe(golden.max_frame_bytes);
  });

  it('encodes every client frame the way Rust decodes it', () => {
    for (const { control, stdin, frame } of golden.client) {
      if (control) {
        // The server parses control JSON by value, so key order is not part of
        // the contract; the channel byte and the parsed object are.
        const encoded = encodeControl(control);
        expect(encoded[0]).toBe(frame[0]);
        expect(JSON.parse(text(encoded.subarray(1)))).toEqual(JSON.parse(text(Uint8Array.from(frame.slice(1)))));
      } else {
        expect(Array.from(encodeStdin(Uint8Array.from(stdin ?? [])))).toEqual(frame);
      }
    }
  });

  it('decodes every server frame the way Rust does, refusals included', () => {
    for (const { frame, expect: want } of golden.server) {
      const decoded = decodeServerFrame(Uint8Array.from(frame).buffer);
      expect(decoded.kind).toBe(want.kind);
      if (decoded.kind === 'output') expect(Array.from(decoded.bytes)).toEqual(want.bytes);
      if (decoded.kind === 'status') expect(decoded.status).toEqual(want.status);
    }
    const oversized = new Uint8Array(golden.oversized_frame_bytes);
    oversized[0] = 1;
    expect(decodeServerFrame(oversized.buffer).kind).toBe('invalid');
  });
});
