// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {PersistRequest} from "../models/PersistRequest.js";
import type {PersistResponse} from "../models/PersistResponse.js";
import {PersistRequestSchema} from "../validation/PersistRequest.js";
import {PersistResponseSchema} from "../validation/PersistResponse.js";

export async function persistVm(
  transport: Transport,
  parameters: {
    "id": string;
    "body": PersistRequest;
  },
  options: CallOptions = {},
): Promise<PersistResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => PersistRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/save", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => PersistResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
