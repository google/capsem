// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {PreviewSessionResponse} from "../models/PreviewSessionResponse.js";
import {PreviewSessionResponseSchema} from "../validation/PreviewSessionResponse.js";

export async function createVmPreviewSession(
  transport: Transport,
  parameters: {
    "id": string;
    "exposure_id": string;
  },
  options: CallOptions = {},
): Promise<PreviewSessionResponse> {
  const input = z.object({
  "id": z.string(),
  "exposure_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/exposures/{exposure_id}/preview-session", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"], "exposure_id": input["exposure_id"]},
  });
  return z.lazy(() => PreviewSessionResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
