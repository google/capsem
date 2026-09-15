// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ProvisionResponse} from "../models/ProvisionResponse.js";
import {ProvisionResponseSchema} from "../validation/ProvisionResponse.js";

export async function startVm(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<ProvisionResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/start", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => ProvisionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
