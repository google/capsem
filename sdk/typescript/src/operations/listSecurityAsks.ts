// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SecurityAskListResponse} from "../models/SecurityAskListResponse.js";
import {SecurityAskListResponseSchema} from "../validation/SecurityAskListResponse.js";

export async function listSecurityAsks(
  transport: Transport,
  parameters: {
    "id": string;
    "limit"?: number;
  },
  options: CallOptions = {},
): Promise<SecurityAskListResponse> {
  const input = z.object({
  "id": z.string(),
  "limit": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/security/asks", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"limit": input["limit"]},
  });
  return z.lazy(() => SecurityAskListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
