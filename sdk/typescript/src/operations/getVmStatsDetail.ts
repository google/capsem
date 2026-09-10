// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {VmStatsDetailResponse} from "../models/VmStatsDetailResponse.js";
import {VmStatsDetailResponseSchema} from "../validation/VmStatsDetailResponse.js";

export async function getVmStatsDetail(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<VmStatsDetailResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/stats/detail", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => VmStatsDetailResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
