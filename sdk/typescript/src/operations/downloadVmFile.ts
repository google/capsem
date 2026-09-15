// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";

export async function downloadVmFile(
  transport: Transport,
  parameters: {
    "id": string;
    "path": string;
  },
  options: CallOptions = {},
): Promise<Uint8Array> {
  const input = z.object({
  "id": z.string(),
  "path": z.string(),
}).parse(parameters);
  return await transport.request(Method.GET, "/vms/{id}/files/content", {
    signal: options.signal, accept: MediaType.BINARY,
    parameters: {"id": input["id"]},
    query: {"path": input["path"]},
  });
}
