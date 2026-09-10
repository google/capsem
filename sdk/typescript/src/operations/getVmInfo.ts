// Generated from Capsem OpenAPI. Do not edit.

import {z} from "zod";
import {Transport, Method, MediaType, type CallOptions} from "../transport.js";
import type {SandboxInfo} from "../models/SandboxInfo.js";
import {SandboxInfoSchema} from "../validation/SandboxInfo.js";

export async function getVmInfo(
  transport: Transport,
  parameters: {
    "id": string;
  },
  options: CallOptions = {},
): Promise<SandboxInfo> {
  const input = z.object({
  "id": z.string(),
}).parse(parameters);
  const payload = await transport.request(Method.GET, "/vms/{id}/info", {
    signal: options.signal, accept: MediaType.JSON,
    parameters: {"id": input["id"]},
  });
  return z.lazy(() => SandboxInfoSchema).parse(JSON.parse(new TextDecoder().decode(payload)));
}
