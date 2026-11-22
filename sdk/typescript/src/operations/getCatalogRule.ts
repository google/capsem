// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {RuleCatalogDetailResponse} from "../models/RuleCatalogDetailResponse.js";
import {RuleCatalogDetailResponseSchema} from "../validation/RuleCatalogDetailResponse.js";

export async function getCatalogRule(
  transport: Transport,
  parameters: {
    "rule_id": string;
    "version": string;
    "snapshot": string;
  },
  options: CallOptions = {},
): Promise<RuleCatalogDetailResponse> {
  const input = z.object({
  "rule_id": z.string(),
  "version": z.string(),
  "snapshot": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/rules/{rule_id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"rule_id": input["rule_id"]},
    query: {"version": input["version"], "snapshot": input["snapshot"]},
  });
  return z.lazy(() => RuleCatalogDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
