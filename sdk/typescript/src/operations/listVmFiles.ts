// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {FileListResponse} from "../models/FileListResponse.js";
import {FileListResponseSchema} from "../validation/FileListResponse.js";

export async function listVmFiles(
  transport: Transport,
  parameters: {
    "id": string;
    "path"?: string;
    "depth"?: number;
  },
  options: CallOptions = {},
): Promise<FileListResponse> {
  const input = z.object({
  "id": z.string(),
  "path": z.string().exactOptional(),
  "depth": z.int().exactOptional(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/files/list", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
    query: {"path": input["path"], "depth": input["depth"]},
  });
  return z.lazy(() => FileListResponseSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
