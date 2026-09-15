// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpRefreshResponse} from "../models/McpRefreshResponse.js";
import {McpRefreshResponseSchema} from "../validation/McpRefreshResponse.js";

export async function refreshProfileMcpServer(
  transport: Transport,
  parameters: {
    "profile_id": string;
    "server_id": string;
  },
  options: CallOptions = {},
): Promise<McpRefreshResponse> {
  const input = z.object({
  "profile_id": z.string(),
  "server_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/profiles/{profile_id}/mcp/servers/{server_id}/refresh", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"profile_id": input["profile_id"], "server_id": input["server_id"]},
  });
  return z.lazy(() => McpRefreshResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
