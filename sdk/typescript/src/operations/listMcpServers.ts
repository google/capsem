// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpServersListResponse} from "../models/McpServersListResponse.js";
import {McpServersListResponseSchema} from "../validation/McpServersListResponse.js";

export async function listMcpServers(
  transport: Transport,
  options: CallOptions = {},
): Promise<McpServersListResponse> {
  const payload = await transport.request(Method.GET, "/mcp/servers/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
  });
  return z.lazy(() => McpServersListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
