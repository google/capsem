// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {PreviewSessionsRevokedResponse} from "../models/PreviewSessionsRevokedResponse.js";
import {PreviewSessionsRevokedResponseSchema} from "../validation/PreviewSessionsRevokedResponse.js";

export async function revokeVmPreviewSessions(
  transport: Transport,
  parameters: {
    "id": string;
    "exposure_id": string;
  },
  options: CallOptions = {},
): Promise<PreviewSessionsRevokedResponse> {
  const input = z.object({
  "id": z.string(),
  "exposure_id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.DELETE, "/vms/{id}/exposures/{exposure_id}/preview-session", {
    signal: options.signal, timeoutMs: options.timeoutMs, accept: MediaType.JSON,
    parameters: {"id": input["id"], "exposure_id": input["exposure_id"]},
  });
  return z.lazy(() => PreviewSessionsRevokedResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
