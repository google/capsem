// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EventBodiesResponse} from "../models/EventBodiesResponse.js";
import {EventBodiesResponseSchema} from "../validation/EventBodiesResponse.js";

export async function getVmEventBodies(
  transport: Transport,
  parameters: {
    "id": string;
    "event_id": string;
    "max_bytes"?: number | null;
  },
  options: CallOptions = {},
): Promise<EventBodiesResponse> {
  const input = z.object({
  "id": z.string(),
  "event_id": z.string(),
  "max_bytes": z.int().min(0).nullable().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/bodies/{event_id}", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "event_id": input["event_id"]},
    query: {"max_bytes": input["max_bytes"]},
  });
  return z.lazy(() => EventBodiesResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
