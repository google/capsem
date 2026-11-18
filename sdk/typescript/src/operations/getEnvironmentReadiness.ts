// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentReadinessResponse} from "../models/EnvironmentReadinessResponse.js";
import {EnvironmentReadinessResponseSchema} from "../validation/EnvironmentReadinessResponse.js";

export async function getEnvironmentReadiness(
  transport: Transport,
  parameters: {
    "id": string;
    "revision_id": string;
  },
  options: CallOptions = {},
): Promise<EnvironmentReadinessResponse> {
  const input = z.object({
  "id": z.string(),
  "revision_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/revisions/{revision_id}/readiness", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "revision_id": input["revision_id"]},
  });
  return z.lazy(() => EnvironmentReadinessResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
