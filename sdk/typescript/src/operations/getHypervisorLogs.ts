// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {HostLogSource} from "../models/HostLogSource.js";
import type {HostLogsResponse} from "../models/HostLogsResponse.js";
import {HostLogSourceSchema} from "../validation/HostLogSource.js";
import {HostLogsResponseSchema} from "../validation/HostLogsResponse.js";

export async function getHypervisorLogs(
  transport: Transport,
  parameters: {
    "name": HostLogSource;
    "grep"?: string;
    "tail"?: number;
    "max_bytes"?: number;
  },
  options: CallOptions = {},
): Promise<HostLogsResponse> {
  const input = z.object({
  "name": z.lazy(() => HostLogSourceSchema),
  "grep": z.string().exactOptional(),
  "tail": z.int().min(0).exactOptional(),
  "max_bytes": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/host-logs/{name}", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"name": input["name"]},
    query: {"grep": input["grep"], "tail": input["tail"], "max_bytes": input["max_bytes"]},
  });
  return z.lazy(() => HostLogsResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
