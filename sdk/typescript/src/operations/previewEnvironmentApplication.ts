// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {EnvironmentApplyPreviewRequest} from "../models/EnvironmentApplyPreviewRequest.js";
import type {EnvironmentApplyPreviewResponse} from "../models/EnvironmentApplyPreviewResponse.js";
import {EnvironmentApplyPreviewRequestSchema} from "../validation/EnvironmentApplyPreviewRequest.js";
import {EnvironmentApplyPreviewResponseSchema} from "../validation/EnvironmentApplyPreviewResponse.js";

export async function previewEnvironmentApplication(
  transport: Transport,
  parameters: {
    "id": string;
    "body": EnvironmentApplyPreviewRequest;
  },
  options: CallOptions = {},
): Promise<EnvironmentApplyPreviewResponse> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => EnvironmentApplyPreviewRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/environments/{id}/apply/preview", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => EnvironmentApplyPreviewResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
