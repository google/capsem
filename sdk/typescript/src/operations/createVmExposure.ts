// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {ExposureInfo} from "../models/ExposureInfo.js";
import type {ExposureRequest} from "../models/ExposureRequest.js";
import {ExposureInfoSchema} from "../validation/ExposureInfo.js";
import {ExposureRequestSchema} from "../validation/ExposureRequest.js";

export async function createVmExposure(
  transport: Transport,
  parameters: {
    "id": string;
    "body": ExposureRequest;
  },
  options: CallOptions = {},
): Promise<ExposureInfo> {
  const input = z.object({
  "id": z.string(),
  "body": z.lazy(() => ExposureRequestSchema),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/exposures", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    body: JSON.stringify(input.body), contentType: MediaType.JSON,
  });
  return z.lazy(() => ExposureInfoSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
