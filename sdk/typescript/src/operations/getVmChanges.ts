// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ChangesResponse} from "../models/ChangesResponse.js";
import {ChangesResponseSchema} from "../validation/ChangesResponse.js";

export async function getVmChanges(
  transport: Transport,
  parameters: {
    "id": string;
    "checkpoint": string;
    "offset"?: number;
    "limit"?: number;
  },
  options: CallOptions = {},
): Promise<ChangesResponse> {
  const input = z.object({
  "id": z.string(),
  "checkpoint": z.string(),
  "offset": z.int().min(0).exactOptional(),
  "limit": z.int().min(0).exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/changes", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"checkpoint": input["checkpoint"], "offset": input["offset"], "limit": input["limit"]},
  });
  return z.lazy(() => ChangesResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
