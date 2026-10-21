// The SDK owns the Rust-generated types and shared MessagePack codec.
export {
  STREAM_SUBPROTOCOL, MAX_STREAM_FRAME_BYTES, MAX_STREAM_DATA_BYTES,
  decodeServerFrame, encodeControl, encodeStdin,
  type StreamControl, type StreamStatus, type ServerFrame,
} from '@capsem/sdk';

export function streamUrl(wsBase: string, vmId: string, token: string): string {
  return `${wsBase}/vms/${encodeURIComponent(vmId)}/stream?token=${encodeURIComponent(token)}`;
}
