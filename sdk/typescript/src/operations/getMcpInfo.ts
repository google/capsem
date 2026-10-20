// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpInfoResponse} from "../models/McpInfoResponse.js";
import {McpInfoResponseSchema} from "../validation/McpInfoResponse.js";

export async function getMcpInfo(
  transport: Transport,
  options: CallOptions = {},
): Promise<McpInfoResponse> {
  const payload = await transport.request(Method.GET, "/mcp/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
  });
  return z.lazy(() => McpInfoResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
