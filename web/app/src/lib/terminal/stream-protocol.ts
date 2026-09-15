// capsem.stream.v1, mirrored from capsem-api's stream module: binary frames
// whose first byte is a channel. Data stays raw bytes; control and status are
// typed JSON. A client only ever decodes server channels.

export const STREAM_SUBPROTOCOL = 'capsem.stream.v1';

const STDIN = 0;
const STDOUT = 1;
const STDERR = 2;
const CONTROL = 3;
const STATUS = 4;

export type StreamControl =
  | { type: 'start'; kind: 'terminal' | 'container' }
  | { type: 'start'; kind: 'exec'; command: string }
  | { type: 'resize'; cols: number; rows: number }
  | { type: 'close_stdin' };

export type StreamStatus =
  | { type: 'started' }
  | { type: 'exit'; code: number; truncated: boolean }
  | { type: 'error'; message: string };

export type ServerFrame =
  | { kind: 'output'; bytes: Uint8Array }
  | { kind: 'status'; status: StreamStatus }
  | { kind: 'invalid'; reason: string };

export function encodeStdin(bytes: Uint8Array): Uint8Array {
  const frame = new Uint8Array(bytes.length + 1);
  frame[0] = STDIN;
  frame.set(bytes, 1);
  return frame;
}

export function encodeControl(control: StreamControl): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(control));
  const frame = new Uint8Array(json.length + 1);
  frame[0] = CONTROL;
  frame.set(json, 1);
  return frame;
}

function isStatus(value: unknown): value is StreamStatus {
  if (typeof value !== 'object' || value === null) return false;
  const status = value as Record<string, unknown>;
  switch (status.type) {
    case 'started':
      return true;
    case 'exit':
      return typeof status.code === 'number' && typeof status.truncated === 'boolean';
    case 'error':
      return typeof status.message === 'string';
    default:
      return false;
  }
}

export function decodeServerFrame(data: ArrayBuffer): ServerFrame {
  const frame = new Uint8Array(data);
  if (frame.length === 0) return { kind: 'invalid', reason: 'empty stream frame' };
  const payload = frame.subarray(1);
  switch (frame[0]) {
    case STDOUT:
    case STDERR:
      return { kind: 'output', bytes: payload };
    case STATUS: {
      try {
        const status: unknown = JSON.parse(new TextDecoder().decode(payload));
        return isStatus(status) ? { kind: 'status', status } : { kind: 'invalid', reason: 'invalid stream status' };
      } catch {
        return { kind: 'invalid', reason: 'invalid stream status' };
      }
    }
    default:
      return { kind: 'invalid', reason: `stream channel ${frame[0]} is not accepted by a client` };
  }
}

export function streamUrl(wsBase: string, vmId: string, token: string): string {
  return `${wsBase}/vms/${encodeURIComponent(vmId)}/stream?token=${encodeURIComponent(token)}`;
}
