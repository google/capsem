// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentApplyRequest} from "../models/EnvironmentApplyRequest.js";
import type {EnvironmentMutationResponse} from "../models/EnvironmentMutationResponse.js";
import {EnvironmentApplyRequestSchema} from "../validation/EnvironmentApplyRequest.js";
import {EnvironmentMutationResponseSchema} from "../validation/EnvironmentMutationResponse.js";

export async function applyEnvironment(
  transport: Transport,
  parameters: {
    "id": string;
    "body": EnvironmentApplyRequest;
  },
  options: CallOptions = {},
): Promise<EnvironmentMutationResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => EnvironmentApplyRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/environments/{id}/apply", {
    signal: options.signal, timeoutMs: options.timeoutMs ?? Math.max(transport.timeoutMs, 1020000), accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentMutationResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
