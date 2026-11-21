// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentRevisionDetailResponse} from "../models/EnvironmentRevisionDetailResponse.js";
import {EnvironmentRevisionDetailResponseSchema} from "../validation/EnvironmentRevisionDetailResponse.js";

export async function getEnvironmentRevision(
  transport: Transport,
  parameters: {
    "id": string;
    "revision_id": string;
  },
  options: CallOptions = {},
): Promise<EnvironmentRevisionDetailResponse> {
  const input = z.object({
  "id": z.string(),
  "revision_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/revisions/{revision_id}/info", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "revision_id": input["revision_id"]},
  });
  return z.lazy(() => EnvironmentRevisionDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
