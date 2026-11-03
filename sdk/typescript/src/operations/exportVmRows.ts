// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";

export async function exportVmRows(
  transport: Transport,
  parameters: {
    "id": string;
    "tables"?: string;
  },
  options: CallOptions = {},
): Promise<Uint8Array> {
  const input = z.object({
  "id": z.string(),
  "tables": z.string().exactOptional(),
}).parse(parameters);
  return await transport.request(Method.GET, "/vms/{id}/export/rows.ndjson.gz", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.GZIP,
    parameters: {"id": input["id"]},
    query: {"tables": input["tables"]},
  });
}
