// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpRefreshResponse} from "../models/McpRefreshResponse.js";
import {McpRefreshResponseSchema} from "../validation/McpRefreshResponse.js";

export async function refreshMcpServer(
  transport: Transport,
  parameters: {
    "server_id": string;
  },
  options: CallOptions = {},
): Promise<McpRefreshResponse> {
  const input = z.object({
  "server_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/mcp/servers/{server_id}/refresh", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"server_id": input["server_id"]},
  });
  return z.lazy(() => McpRefreshResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
