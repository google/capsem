// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ProvisionRequest} from "../models/ProvisionRequest.js";
import type {ProvisionResponse} from "../models/ProvisionResponse.js";
import {ProvisionRequestSchema} from "../validation/ProvisionRequest.js";
import {ProvisionResponseSchema} from "../validation/ProvisionResponse.js";

export async function createVm(
  transport: Transport,
  parameters: {
    "body": ProvisionRequest;
  },
  options: CallOptions = {},
): Promise<ProvisionResponse> {
  const input = z.object({
  "body": z.lazy(() => ProvisionRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/create", {
    signal: options.signal, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ProvisionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
