// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {RuleCatalogListResponse} from "../models/RuleCatalogListResponse.js";
import {RuleCatalogListResponseSchema} from "../validation/RuleCatalogListResponse.js";

export async function listCatalogRules(
  transport: Transport,
  parameters: {
    "search"?: string;
    "after"?: string;
    "limit"?: number;
  } = {},
  options: CallOptions = {},
): Promise<RuleCatalogListResponse> {
  const input = z.object({
  "search": z.string().exactOptional(),
  "after": z.string().exactOptional(),
  "limit": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/rules/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    query: {"search": input["search"], "after": input["after"], "limit": input["limit"]},
  });
  return z.lazy(() => RuleCatalogListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
