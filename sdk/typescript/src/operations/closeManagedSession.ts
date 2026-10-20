// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ManagedSessionCloseRequest} from "../models/ManagedSessionCloseRequest.js";
import type {ManagedSessionCloseResponse} from "../models/ManagedSessionCloseResponse.js";
import {ManagedSessionCloseRequestSchema} from "../validation/ManagedSessionCloseRequest.js";
import {ManagedSessionCloseResponseSchema} from "../validation/ManagedSessionCloseResponse.js";

export async function closeManagedSession(
  transport: Transport,
  parameters: {
    "request_id": string;
    "body": ManagedSessionCloseRequest;
  },
  options: CallOptions = {},
): Promise<ManagedSessionCloseResponse> {
  const input = z.object({
  "request_id": z.string(),
  "body": z.lazy(() => ManagedSessionCloseRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/managed/{request_id}/close", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"request_id": input["request_id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ManagedSessionCloseResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
