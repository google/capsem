// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {McpCatalogListResponse} from "../models/McpCatalogListResponse.js";
import {McpCatalogListResponseSchema} from "../validation/McpCatalogListResponse.js";

export async function listCatalogMcps(
  transport: Transport,
  parameters: {
    "search"?: string;
    "after"?: string;
    "limit"?: number;
  } = {},
  options: CallOptions = {},
): Promise<McpCatalogListResponse> {
  const input = z.object({
  "search": z.string().exactOptional(),
  "after": z.string().exactOptional(),
  "limit": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/mcps/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    query: {"search": input["search"], "after": input["after"], "limit": input["limit"]},
  });
  return z.lazy(() => McpCatalogListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
