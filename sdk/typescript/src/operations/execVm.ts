// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ExecRequest} from "../models/ExecRequest.js";
import type {ExecResponse} from "../models/ExecResponse.js";
import {ExecRequestSchema} from "../validation/ExecRequest.js";
import {ExecResponseSchema} from "../validation/ExecResponse.js";

export async function execVm(
  transport: Transport,
  parameters: {
    "id": string;
    "body": ExecRequest;
  },
  options: CallOptions = {},
): Promise<ExecResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => ExecRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/exec", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ExecResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
