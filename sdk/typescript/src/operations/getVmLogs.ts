// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {LogsResponse} from "../models/LogsResponse.js";
import {LogsResponseSchema} from "../validation/LogsResponse.js";

export async function getVmLogs(
  transport: Transport,
  parameters: {
    "id": string;
    "grep"?: string;
    "tail"?: number;
    "max_bytes"?: number;
  },
  options: CallOptions = {},
): Promise<LogsResponse> {
  const input = z.object({
  "id": z.string(),
  "grep": z.string().exactOptional(),
  "tail": z.int().min(0).exactOptional(),
  "max_bytes": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/logs", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"grep": input["grep"], "tail": input["tail"], "max_bytes": input["max_bytes"]},
  });
  return z.lazy(() => LogsResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
