// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {VmMcpToolsResponse} from "../models/VmMcpToolsResponse.js";
import {VmMcpToolsResponseSchema} from "../validation/VmMcpToolsResponse.js";

export async function listVmMcpTools(
  transport: Transport,
  parameters: {
    "id": string;
    "server_id": string;
  },
  options: CallOptions = {},
): Promise<VmMcpToolsResponse> {
  const input = z.object({
  "id": z.string(),
  "server_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/mcps/{server_id}/tools", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "server_id": input["server_id"]},
  });
  return z.lazy(() => VmMcpToolsResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
