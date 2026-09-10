// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {TimelineLayer} from "../models/TimelineLayer.js";
import type {TimelineResponse} from "../models/TimelineResponse.js";
import {TimelineLayerSchema} from "../validation/TimelineLayer.js";
import {TimelineResponseSchema} from "../validation/TimelineResponse.js";

export async function getVmTimeline(
  transport: Transport,
  parameters: {
    "id": string;
    "trace_id"?: string;
    "since"?: string;
    "limit"?: number;
    "layers"?: Array<TimelineLayer>;
  },
  options: CallOptions = {},
): Promise<TimelineResponse> {
  const input = z.object({
  "id": z.string(),
  "trace_id": z.string().exactOptional(),
  "since": z.string().exactOptional(),
  "limit": z.int().min(0).exactOptional(),
  "layers": z.array(z.lazy(() => TimelineLayerSchema)).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/timeline", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"trace_id": input["trace_id"], "since": input["since"], "limit": input["limit"], "layers": input["layers"]},
  });
  return z.lazy(() => TimelineResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
