// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {StopResponse} from "../models/StopResponse.js";
import {StopResponseSchema} from "../validation/StopResponse.js";

export async function stopVm(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<StopResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/stop", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => StopResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
