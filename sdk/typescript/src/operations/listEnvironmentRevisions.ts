// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentRevisionListResponse} from "../models/EnvironmentRevisionListResponse.js";
import {EnvironmentRevisionListResponseSchema} from "../validation/EnvironmentRevisionListResponse.js";

export async function listEnvironmentRevisions(
  transport: Transport,
  parameters: {
    "id": string;
    "before"?: number;
    "limit"?: number;
  },
  options: CallOptions = {},
): Promise<EnvironmentRevisionListResponse> {
  const input = z.object({
  "id": z.string(),
  "before": z.int().exactOptional(),
  "limit": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/revisions/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"before": input["before"], "limit": input["limit"]},
  });
  return z.lazy(() => EnvironmentRevisionListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
