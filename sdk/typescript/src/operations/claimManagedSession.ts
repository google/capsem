// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ManagedSessionClaimRequest} from "../models/ManagedSessionClaimRequest.js";
import type {ManagedSessionClaimResponse} from "../models/ManagedSessionClaimResponse.js";
import {ManagedSessionClaimRequestSchema} from "../validation/ManagedSessionClaimRequest.js";
import {ManagedSessionClaimResponseSchema} from "../validation/ManagedSessionClaimResponse.js";

export async function claimManagedSession(
  transport: Transport,
  parameters: {
    "request_id": string;
    "body": ManagedSessionClaimRequest;
  },
  options: CallOptions = {},
): Promise<ManagedSessionClaimResponse> {
  const input = z.object({
  "request_id": z.string(),
  "body": z.lazy(() => ManagedSessionClaimRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/managed/{request_id}/claim", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"request_id": input["request_id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ManagedSessionClaimResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
