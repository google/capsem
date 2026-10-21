/** The capsem-proto named MessagePack contract; WebSocket delimits records. */
import {decode, encode} from '@msgpack/msgpack';
import {StreamKind, TerminalEventKind} from './models/index.js';
import type {StreamControl, StreamStatus, TerminalNotification, TerminalState} from './models/index.js';
import {StreamControlSchema} from './validation/StreamControl.js';
import {StreamStatusSchema} from './validation/StreamStatus.js';

export const STREAM_SUBPROTOCOL = 'capsem.stream.v3';
export const MAX_STREAM_FRAME_BYTES = 512 * 1024;
export const MAX_STREAM_DATA_BYTES = 256 * 1024 - 1;

export type ServerFrame =
  | {kind: 'output'; bytes: Uint8Array}
  | {kind: 'status'; status: StreamStatus}
  | {kind: 'invalid'; reason: string};

function encodeRecord(record: object): Uint8Array {
  const bytes = encode(record);
  if (bytes.length > MAX_STREAM_FRAME_BYTES) throw new Error('stream frame exceeds its budget');
  return bytes;
}

export function encodeStdin(bytes: Uint8Array): Uint8Array {
  if (bytes.length > MAX_STREAM_DATA_BYTES) throw new Error('stream data exceeds its budget');
  return encodeRecord({stdin: bytes});
}

function size(cols: number, rows: number): void {
  if (cols * rows > 65536) throw new Error('terminal dimensions exceed the supported grid');
}

export function encodeControl(value: StreamControl): Uint8Array {
  const control = StreamControlSchema.parse(value);
  if (control.type === 'resize' || control.type === 'focus') size(control.cols, control.rows);
  if (control.type === 'start') {
    if (control.kind === StreamKind.EXEC ? !control.command : control.command != null) {
      throw new Error('only exec takes a required nonempty command');
    }
  }
  return encodeRecord({control});
}

function text(value: string): void {
  if (new TextEncoder().encode(value).length > 1024 || Array.from(value).some(char => {
    const code = char.codePointAt(0) ?? 0;
    return code < 32 || (code >= 127 && code <= 159);
  })) {
    throw new Error('terminal metadata exceeds the supported text budget');
  }
}

function notification(value: TerminalNotification): void {
  text(value.id);
  text(value.title);
  text(value.body);
}

function terminal(state: TerminalState): void {
  size(state.cols, state.rows);
  if (state.cursor.row >= state.rows || state.cursor.col > state.cols) {
    throw new Error('terminal cursor is outside the supported grid');
  }
  for (const value of [state.title, state.icon_name, state.modes.mouse_protocol, state.modes.mouse_encoding]) text(value);
  if (state.last_notification) notification(state.last_notification);
  for (const event of state.events) {
    if (event.kind === TerminalEventKind.NOTIFICATION && event.notification) notification(event.notification);
    else if (event.kind !== TerminalEventKind.BELL || event.notification) throw new Error('terminal event payload does not match its kind');
  }
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) && !(value instanceof Uint8Array);
}

// Match the Rust decoder's nesting limit and refuse numbers JavaScript cannot
// represent exactly. Binary payloads are leaves, never arrays of numbers.
function boundedTree(value: unknown, depth = 0): void {
  if (depth > 64) throw new Error('stream nesting exceeds its budget');
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new Error('stream integer is not exact');
  if (value instanceof Uint8Array) return;
  if (typeof value === 'object' && value !== null) {
    for (const child of Object.values(value)) boundedTree(child, depth + 1);
  }
}

export function decodeServerFrame(data: ArrayBuffer | Uint8Array): ServerFrame {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
  try {
    if (!bytes.length || bytes.length > MAX_STREAM_FRAME_BYTES) throw new Error('stream frame exceeds its budget');
    const value: unknown = decode(bytes, {
      maxBinLength: MAX_STREAM_DATA_BYTES, maxStrLength: MAX_STREAM_FRAME_BYTES,
      maxArrayLength: 32, maxMapLength: 32, maxExtLength: 0,
      mapKeyConverter: key => {
        if (typeof key !== 'string') throw new Error('stream map keys must be strings');
        return key;
      },
    });
    boundedTree(value);
    if (!record(value) || Object.keys(value).length !== 1) throw new Error('invalid stream record');
    if ('stdout' in value || 'stderr' in value) {
      const output = value.stdout ?? value.stderr;
      if (!(output instanceof Uint8Array)) throw new Error('stream data must be binary');
      return {kind: 'output', bytes: output};
    }
    if (!('status' in value)) throw new Error('stream message has the wrong direction');
    const status = StreamStatusSchema.parse(value.status);
    if (status.type === 'terminal') terminal(status.state);
    if (status.type === 'exit' && (status.code < -2147483648 || status.code > 2147483647)) {
      throw new Error('stream exit status exceeds i32');
    }
    return {kind: 'status', status};
  } catch (error) {
    return {kind: 'invalid', reason: error instanceof Error ? error.message : 'invalid stream record'};
  }
}
