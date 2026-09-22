// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {TriageResponse} from "../models/TriageResponse.js";
import {TriageResponseSchema} from "../validation/TriageResponse.js";

export async function getTriage(
  transport: Transport,
  parameters: {
    "since"?: string | null;
    "limit"?: number | null;
    "id"?: string | null;
  } = {},
  options: CallOptions = {},
): Promise<TriageResponse> {
  const input = z.object({
  "since": z.string().nullable().exactOptional(),
  "limit": z.int().min(0).nullable().exactOptional(),
  "id": z.string().nullable().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/triage", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    query: {"since": input["since"], "limit": input["limit"], "id": input["id"]},
  });
  return z.lazy(() => TriageResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
