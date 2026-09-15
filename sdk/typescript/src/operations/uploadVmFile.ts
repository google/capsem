// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {UploadResponse} from "../models/UploadResponse.js";
import {UploadResponseSchema} from "../validation/UploadResponse.js";

export async function uploadVmFile(
  transport: Transport,
  parameters: {
    "id": string;
    "path": string;
    "body": Uint8Array;
  },
  options: CallOptions = {},
): Promise<UploadResponse> {
  const input = z.object({
  "id": z.string(),
  "path": z.string(),
  "body": z.instanceof(Uint8Array),
}).parse(parameters);
  const payload = await transport.request(Method.POST, "/vms/{id}/files/content", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"path": input["path"]},
    body: input.body, contentType: MediaType.BINARY,
  });
  return z.lazy(() => UploadResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
