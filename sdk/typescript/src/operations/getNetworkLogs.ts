// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {NetworkLogsResponse} from "../models/NetworkLogsResponse.js";
import {NetworkLogsResponseSchema} from "../validation/NetworkLogsResponse.js";

export async function getNetworkLogs(
  transport: Transport,
  parameters: {
    "id": string;
    "cursor"?: string | null;
    "limit"?: number | null;
    "vm"?: string | null;
    "connection"?: string | null;
    "type"?: string | null;
    "decision"?: string | null;
    "since"?: number | null;
    "until"?: number | null;
  },
  options: CallOptions = {},
): Promise<NetworkLogsResponse> {
  const input = z.object({
  "id": z.string(),
  "cursor": z.string().nullable().exactOptional(),
  "limit": z.int().min(0).nullable().exactOptional(),
  "vm": z.string().nullable().exactOptional(),
  "connection": z.string().nullable().exactOptional(),
  "type": z.string().nullable().exactOptional(),
  "decision": z.string().nullable().exactOptional(),
  "since": z.int().nullable().exactOptional(),
  "until": z.int().nullable().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/networks/{id}/logs", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"cursor": input["cursor"], "limit": input["limit"], "vm": input["vm"], "connection": input["connection"], "type": input["type"], "decision": input["decision"], "since": input["since"], "until": input["until"]},
  });
  return z.lazy(() => NetworkLogsResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
