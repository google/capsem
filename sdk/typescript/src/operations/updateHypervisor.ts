// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {UpdateActionResponse} from "../models/UpdateActionResponse.js";
import type {UpdateApplyRequest} from "../models/UpdateApplyRequest.js";
import {UpdateActionResponseSchema} from "../validation/UpdateActionResponse.js";
import {UpdateApplyRequestSchema} from "../validation/UpdateApplyRequest.js";

export async function updateHypervisor(
  transport: Transport,
  parameters: {
    "body": UpdateApplyRequest;
  },
  options: CallOptions = {},
): Promise<UpdateActionResponse> {
  const input = z.object({
  "body": z.lazy(() => UpdateApplyRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/update/apply", {
    signal: options.signal, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => UpdateActionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
