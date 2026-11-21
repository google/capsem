// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {RequestListResponse} from "../models/RequestListResponse.js";
import {RequestListResponseSchema} from "../validation/RequestListResponse.js";

export async function listRequests(
  transport: Transport,
  parameters: {
    "after"?: number;
    "limit"?: number;
    "pending_only"?: boolean;
  } = {},
  options: CallOptions = {},
): Promise<RequestListResponse> {
  const input = z.object({
  "after": z.int().exactOptional(),
  "limit": z.int().min(0).exactOptional(),
  "pending_only": z.boolean().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/requests/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    query: {"after": input["after"], "limit": input["limit"], "pending_only": input["pending_only"]},
  });
  return z.lazy(() => RequestListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
