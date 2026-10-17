// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ConnectionSummary} from "../models/ConnectionSummary.js";
import {ConnectionSummarySchema} from "../validation/ConnectionSummary.js";

export async function getConnection(
  transport: Transport,
  parameters: {
    "connection_id": string;
  },
  options: CallOptions = {},
): Promise<ConnectionSummary> {
  const input = z.object({
  "connection_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/connections/{connection_id}", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"connection_id": input["connection_id"]},
  });
  return z.lazy(() => ConnectionSummarySchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
