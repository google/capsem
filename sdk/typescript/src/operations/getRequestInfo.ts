// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {RequestDetailResponse} from "../models/RequestDetailResponse.js";
import {RequestDetailResponseSchema} from "../validation/RequestDetailResponse.js";

export async function getRequestInfo(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<RequestDetailResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/requests/{id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => RequestDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
