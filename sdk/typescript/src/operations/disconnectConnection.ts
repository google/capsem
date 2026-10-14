// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {DisconnectResponse} from "../models/DisconnectResponse.js";
import {DisconnectResponseSchema} from "../validation/DisconnectResponse.js";

export async function disconnectConnection(
  transport: Transport,
  parameters: {
    "connection_id": string;
  },
  options: CallOptions = {},
): Promise<DisconnectResponse> {
  const input = z.object({
  "connection_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.DELETE, "/connections/{connection_id}", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"connection_id": input["connection_id"]},
  });
  return z.lazy(() => DisconnectResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
