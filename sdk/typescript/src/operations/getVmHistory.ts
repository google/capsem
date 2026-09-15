// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {HistoryLayerFilter} from "../models/HistoryLayerFilter.js";
import type {HistoryResponse} from "../models/HistoryResponse.js";
import {HistoryLayerFilterSchema} from "../validation/HistoryLayerFilter.js";
import {HistoryResponseSchema} from "../validation/HistoryResponse.js";

export async function getVmHistory(
  transport: Transport,
  parameters: {
    "id": string;
    "limit"?: number;
    "offset"?: number;
    "search"?: string;
    "layer"?: HistoryLayerFilter;
  },
  options: CallOptions = {},
): Promise<HistoryResponse> {
  const input = z.object({
  "id": z.string(),
  "limit": z.int().min(0).exactOptional(),
  "offset": z.int().min(0).exactOptional(),
  "search": z.string().exactOptional(),
  "layer": z.lazy(() => HistoryLayerFilterSchema).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/history", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"limit": input["limit"], "offset": input["offset"], "search": input["search"], "layer": input["layer"]},
  });
  return z.lazy(() => HistoryResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
