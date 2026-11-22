// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpCatalogDetailResponse} from "../models/McpCatalogDetailResponse.js";
import {McpCatalogDetailResponseSchema} from "../validation/McpCatalogDetailResponse.js";

export async function getCatalogMcp(
  transport: Transport,
  parameters: {
    "mcp_id": string;
    "version": string;
    "snapshot": string;
  },
  options: CallOptions = {},
): Promise<McpCatalogDetailResponse> {
  const input = z.object({
  "mcp_id": z.string(),
  "version": z.string(),
  "snapshot": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/mcps/{mcp_id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"mcp_id": input["mcp_id"]},
    query: {"version": input["version"], "snapshot": input["snapshot"]},
  });
  return z.lazy(() => McpCatalogDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
