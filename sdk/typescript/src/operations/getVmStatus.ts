// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {VmStatusResponse} from "../models/VmStatusResponse.js";
import {VmStatusResponseSchema} from "../validation/VmStatusResponse.js";

export async function getVmStatus(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<VmStatusResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/status", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => VmStatusResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
