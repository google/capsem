// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpToolsListResponse} from "../models/McpToolsListResponse.js";
import {McpToolsListResponseSchema} from "../validation/McpToolsListResponse.js";

export async function listProfileMcpTools(
  transport: Transport,
  parameters: {
    "profile_id": string;
    "server_id": string;
  },
  options: CallOptions = {},
): Promise<McpToolsListResponse> {
  const input = z.object({
  "profile_id": z.string(),
  "server_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/profiles/{profile_id}/mcp/servers/{server_id}/tools/list", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"profile_id": input["profile_id"], "server_id": input["server_id"]},
  });
  return z.lazy(() => McpToolsListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
