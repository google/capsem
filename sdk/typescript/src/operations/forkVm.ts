// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ForkRequest} from "../models/ForkRequest.js";
import type {ForkResponse} from "../models/ForkResponse.js";
import {ForkRequestSchema} from "../validation/ForkRequest.js";
import {ForkResponseSchema} from "../validation/ForkResponse.js";

export async function forkVm(
  transport: Transport,
  parameters: {
    "id": string;
    "body": ForkRequest;
  },
  options: CallOptions = {},
): Promise<ForkResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => ForkRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/fork", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ForkResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
