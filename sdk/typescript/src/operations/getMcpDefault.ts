// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpDefaultPermissionResponse} from "../models/McpDefaultPermissionResponse.js";
import {McpDefaultPermissionResponseSchema} from "../validation/McpDefaultPermissionResponse.js";

export async function getMcpDefault(
  transport: Transport,
  options: CallOptions = {},
): Promise<McpDefaultPermissionResponse> {
  const payload = await transport.request(Method.GET, "/mcp/default/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
  });
  return z.lazy(() => McpDefaultPermissionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
