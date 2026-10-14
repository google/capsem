// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ConnectionListResponse} from "../models/ConnectionListResponse.js";
import {ConnectionListResponseSchema} from "../validation/ConnectionListResponse.js";

export async function listConnections(
  transport: Transport,
  options: CallOptions = {},
): Promise<ConnectionListResponse> {
  const payload = await transport.request(Method.GET, "/connections", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
  });
  return z.lazy(() => ConnectionListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
