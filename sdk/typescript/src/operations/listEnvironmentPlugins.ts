// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentPluginListResponse} from "../models/EnvironmentPluginListResponse.js";
import {EnvironmentPluginListResponseSchema} from "../validation/EnvironmentPluginListResponse.js";

export async function listEnvironmentPlugins(
  transport: Transport,
  parameters: {
    "id": string;
    "revision_id"?: string | null;
  },
  options: CallOptions = {},
): Promise<EnvironmentPluginListResponse> {
  const input = z.object({
  "id": z.string(),
  "revision_id": z.uuid().nullable().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/environments/{id}/plugins/list", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"revision_id": input["revision_id"]},
  });
  return z.lazy(() => EnvironmentPluginListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
