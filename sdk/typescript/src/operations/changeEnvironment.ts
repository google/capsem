// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentChangeRequest} from "../models/EnvironmentChangeRequest.js";
import type {EnvironmentMutationResponse} from "../models/EnvironmentMutationResponse.js";
import {EnvironmentChangeRequestSchema} from "../validation/EnvironmentChangeRequest.js";
import {EnvironmentMutationResponseSchema} from "../validation/EnvironmentMutationResponse.js";

export async function changeEnvironment(
  transport: Transport,
  parameters: {
    "id": string;
    "body": EnvironmentChangeRequest;
  },
  options: CallOptions = {},
): Promise<EnvironmentMutationResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => EnvironmentChangeRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/environments/{id}/changes", {
    signal: options.signal, timeoutMs: options.timeoutMs ?? Math.max(transport.timeoutMs, 1020000), accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentMutationResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
