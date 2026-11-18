// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentCreateRequest} from "../models/EnvironmentCreateRequest.js";
import type {EnvironmentMutationResponse} from "../models/EnvironmentMutationResponse.js";
import {EnvironmentCreateRequestSchema} from "../validation/EnvironmentCreateRequest.js";
import {EnvironmentMutationResponseSchema} from "../validation/EnvironmentMutationResponse.js";

export async function createEnvironment(
  transport: Transport,
  parameters: {
    "body": EnvironmentCreateRequest;
  },
  options: CallOptions = {},
): Promise<EnvironmentMutationResponse> {
  const input = z.object({
  "body": z.lazy(() => EnvironmentCreateRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/environments/create", {
    signal: options.signal, timeoutMs: options.timeoutMs ?? Math.max(transport.timeoutMs, 1020000), accept: MediaType.JSON,
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentMutationResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
