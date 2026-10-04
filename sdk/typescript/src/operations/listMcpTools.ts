// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpToolsListResponse} from "../models/McpToolsListResponse.js";
import {McpToolsListResponseSchema} from "../validation/McpToolsListResponse.js";

export async function listMcpTools(
  transport: Transport,
  parameters: {
    "server_id": string;
  },
  options: CallOptions = {},
): Promise<McpToolsListResponse> {
  const input = z.object({
  "server_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/mcp/servers/{server_id}/tools/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"server_id": input["server_id"]},
  });
  return z.lazy(() => McpToolsListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
