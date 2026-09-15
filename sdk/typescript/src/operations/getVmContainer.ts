// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ContainerStatusResponse} from "../models/ContainerStatusResponse.js";
import {ContainerStatusResponseSchema} from "../validation/ContainerStatusResponse.js";

export async function getVmContainer(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<ContainerStatusResponse> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/container", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => ContainerStatusResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
