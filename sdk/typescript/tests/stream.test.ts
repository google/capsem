import {readFileSync} from 'node:fs';
import {decode, encode} from '@msgpack/msgpack';
import {describe, expect, it} from 'vitest';
import {StreamKind} from '../src/models/index.js';
import type {StreamControl} from '../src/models/index.js';
import {decodeServerFrame, encodeControl, encodeStdin, MAX_STREAM_DATA_BYTES,
  MAX_STREAM_FRAME_BYTES, STREAM_SUBPROTOCOL} from '../src/stream.js';

interface Golden {
  subprotocol: string;
  max_frame_bytes: number;
  max_data_bytes: number;
  client: Array<{control?: StreamControl; stdin?: number[]; frame: number[]}>;
  server: Array<{frame: number[]; expect: {kind: string; status?: unknown; bytes?: number[]}}>;
}
const golden = JSON.parse(readFileSync(new URL('../../specification/stream-v3.json', import.meta.url), 'utf8')) as Golden;
const terminal = golden.server.find(entry => entry.expect.status &&
  (entry.expect.status as {type: string}).type === 'terminal')?.expect.status as {type: 'terminal'; state: Record<string, unknown>};

describe('the shared Rust MessagePack contract', () => {
  it('matches the protocol, binary representation and every Rust fixture', () => {
    expect(STREAM_SUBPROTOCOL).toBe(golden.subprotocol);
    expect(MAX_STREAM_FRAME_BYTES).toBe(golden.max_frame_bytes);
    expect(MAX_STREAM_DATA_BYTES).toBe(golden.max_data_bytes);
    for (const entry of golden.client) {
      if (entry.control) expect(decode(encodeControl(entry.control))).toEqual(decode(Uint8Array.from(entry.frame)));
      else expect([...encodeStdin(Uint8Array.from(entry.stdin ?? []))]).toEqual(entry.frame);
    }
    for (const entry of golden.server) {
      const result = decodeServerFrame(Uint8Array.from(entry.frame));
      expect(result.kind).toBe(entry.expect.kind);
      if (result.kind === 'status') expect(result.status).toEqual(entry.expect.status);
      if (result.kind === 'output') expect([...result.bytes]).toEqual(entry.expect.bytes);
    }
  });

  it('enforces size and control bounds before sending', () => {
    expect(() => encodeStdin(new Uint8Array(MAX_STREAM_DATA_BYTES + 1))).toThrow();
    expect(() => encodeControl({type: 'resize', cols: 512, rows: 256})).toThrow();
    expect(() => encodeControl({type: 'focus', cols: 0, rows: 24})).toThrow();
    expect(() => encodeControl({type: 'start', kind: StreamKind.EXEC})).toThrow();
    expect(() => encodeControl({type: 'start', kind: StreamKind.TERMINAL, command: 'wrong'})).toThrow();
    expect(() => encodeControl({type: 'start', kind: StreamKind.EXEC, command: 'x'.repeat(MAX_STREAM_FRAME_BYTES)})).toThrow();
    expect(decodeServerFrame(encode({stdout: new Uint8Array(MAX_STREAM_DATA_BYTES)})).kind).toBe('output');
    expect(decodeServerFrame(encode({stdout: new Uint8Array(MAX_STREAM_DATA_BYTES + 1)})).kind).toBe('invalid');
    expect(decodeServerFrame(new Uint8Array(MAX_STREAM_FRAME_BYTES + 1)).kind).toBe('invalid');
  });

  it('rejects legacy framing, nonbinary data, malformed and trailing records', () => {
    for (const value of [{stdout: [1, 2]}, {stdout: 'text'}, {stdin: new Uint8Array()},
      {stdout: new Uint8Array(), stderr: new Uint8Array()}, {status: {type: 'exit', code: 2 ** 31, truncated: false}},
      {status: {type: 'exit'}}, {status: null}, {status: {type: 'unknown'}}]) {
      expect(decodeServerFrame(encode(value)).kind).toBe('invalid');
    }
    for (const bytes of [new Uint8Array(), new Uint8Array([1, 65]), new Uint8Array([0xc1]),
      new Uint8Array([...encode({status: {type: 'started'}}), 0]), new Uint8Array([0x81])]) {
      expect(decodeServerFrame(bytes).kind).toBe('invalid');
    }
    let nested: unknown = 0;
    for (let index = 0; index < 70; index++) nested = [nested];
    expect(decodeServerFrame(encode({status: {type: 'started', nested}})).kind).toBe('invalid');
  });

  it('refuses invalid terminal state before the renderer can allocate', () => {
    for (const change of [{cols: 0}, {cols: 512, rows: 256}, {title: '\u0007'},
      {title: '😀'.repeat(300)}, {cursor: {row: 40, col: 0, hidden: false}},
      {revision: Number.MAX_SAFE_INTEGER + 1},
      {events: [{kind: 'notification', visual: false}]},
      {events: [{kind: 'bell', visual: false, notification: {id: 'x', title: '', body: ''}}]},
      {last_notification: {id: '', title: '', body: '\u0085'}},
      {events: Array.from({length: 33}, () => ({kind: 'bell', visual: false}))}]) {
      expect(decodeServerFrame(encode({status: {...terminal, state: {...terminal.state, ...change}}})).kind).toBe('invalid');
    }
  });
});
