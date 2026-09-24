// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";

export async function exportVmBodies(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<Uint8Array> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  return await transport.request(Method.GET, "/vms/{id}/bodies/export.warc.gz", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.GZIP,
    parameters: {"id": input["id"]},
  });
}
