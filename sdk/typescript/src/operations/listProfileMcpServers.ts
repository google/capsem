// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpServersListResponse} from "../models/McpServersListResponse.js";
import {McpServersListResponseSchema} from "../validation/McpServersListResponse.js";

export async function listProfileMcpServers(
  transport: Transport,
  parameters: {
    "profile_id": string;
  },
  options: CallOptions = {},
): Promise<McpServersListResponse> {
  const input = z.object({
  "profile_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/profiles/{profile_id}/mcp/servers/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"profile_id": input["profile_id"]},
  });
  return z.lazy(() => McpServersListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
