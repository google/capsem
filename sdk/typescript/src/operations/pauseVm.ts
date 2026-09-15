// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {VmActionResponse} from "../models/VmActionResponse.js";
import {VmActionResponseSchema} from "../validation/VmActionResponse.js";

export async function pauseVm(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<VmActionResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/pause", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => VmActionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
