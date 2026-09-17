// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ExposureListResponse} from "../models/ExposureListResponse.js";
import {ExposureListResponseSchema} from "../validation/ExposureListResponse.js";

export async function listVmExposures(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<ExposureListResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/exposures", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => ExposureListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
