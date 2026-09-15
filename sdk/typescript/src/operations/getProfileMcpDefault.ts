// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpDefaultPermissionResponse} from "../models/McpDefaultPermissionResponse.js";
import {McpDefaultPermissionResponseSchema} from "../validation/McpDefaultPermissionResponse.js";

export async function getProfileMcpDefault(
  transport: Transport,
  parameters: {
    "profile_id": string;
  },
  options: CallOptions = {},
): Promise<McpDefaultPermissionResponse> {
  const input = z.object({
  "profile_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/profiles/{profile_id}/mcp/default/info", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"profile_id": input["profile_id"]},
  });
  return z.lazy(() => McpDefaultPermissionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
