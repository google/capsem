// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentMutationResponse} from "../models/EnvironmentMutationResponse.js";
import type {RequestResolveRequest} from "../models/RequestResolveRequest.js";
import {EnvironmentMutationResponseSchema} from "../validation/EnvironmentMutationResponse.js";
import {RequestResolveRequestSchema} from "../validation/RequestResolveRequest.js";

export async function resolveRequest(
  transport: Transport,
  parameters: {
    "id": string;
    "body": RequestResolveRequest;
  },
  options: CallOptions = {},
): Promise<EnvironmentMutationResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => RequestResolveRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/requests/{id}/resolve", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentMutationResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
