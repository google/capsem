// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {UpdateActionResponse} from "../models/UpdateActionResponse.js";
import type {UpdateCheckRequest} from "../models/UpdateCheckRequest.js";
import {UpdateActionResponseSchema} from "../validation/UpdateActionResponse.js";
import {UpdateCheckRequestSchema} from "../validation/UpdateCheckRequest.js";

export async function checkHypervisorUpdate(
  transport: Transport,
  parameters: {
    "body": UpdateCheckRequest;
  },
  options: CallOptions = {},
): Promise<UpdateActionResponse> {
  const input = z.object({
  "body": z.lazy(() => UpdateCheckRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/update/check", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => UpdateActionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
