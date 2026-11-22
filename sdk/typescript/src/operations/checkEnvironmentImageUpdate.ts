// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentImageUpdateResponse} from "../models/EnvironmentImageUpdateResponse.js";
import {EnvironmentImageUpdateResponseSchema} from "../validation/EnvironmentImageUpdateResponse.js";

export async function checkEnvironmentImageUpdate(
  transport: Transport,
  parameters: {
    "id": string;
    "revision_id": string;
  },
  options: CallOptions = {},
): Promise<EnvironmentImageUpdateResponse> {
  const input = z.object({
  "id": z.string(),
  "revision_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/revisions/{revision_id}/image-update", {
    signal: options.signal, timeoutMs: options.timeoutMs ?? Math.max(transport.timeoutMs, 1020000), accept: MediaType.JSON,
    parameters: {"id": input["id"], "revision_id": input["revision_id"]},
  });
  return z.lazy(() => EnvironmentImageUpdateResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
