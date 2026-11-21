// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentTomlValidationRequest} from "../models/EnvironmentTomlValidationRequest.js";
import type {EnvironmentTomlValidationResponse} from "../models/EnvironmentTomlValidationResponse.js";
import {EnvironmentTomlValidationRequestSchema} from "../validation/EnvironmentTomlValidationRequest.js";
import {EnvironmentTomlValidationResponseSchema} from "../validation/EnvironmentTomlValidationResponse.js";

export async function validateEnvironmentToml(
  transport: Transport,
  parameters: {
    "id": string;
    "revision_id": string;
    "body": EnvironmentTomlValidationRequest;
  },
  options: CallOptions = {},
): Promise<EnvironmentTomlValidationResponse> {
  const input = z.object({
  "id": z.string(),
  "revision_id": z.string(),
  "body": z.lazy(() => EnvironmentTomlValidationRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/environments/{id}/revisions/{revision_id}/validate", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "revision_id": input["revision_id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentTomlValidationResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
